//! `bevy_fairy/ui_text`: what text the player can read right now.
//!
//! A claim like "this sentence is on screen once, not twice" is a claim about text, and the game
//! knows exactly which text it drew. A raw `world.query` over `Text` answers it badly: it returns
//! every `Text` node in the tree (pooled labels, toast slots that expired and kept their old string),
//! each with `InheritedVisibility` still true and no rectangle. This command does the joining a
//! reader would otherwise do by hand, and says *why* a node is hidden rather than only that it is.
//!
//! Scope: `bevy_ui` text (`Text` and its `TextSpan` children). `Text2d` and world-space text are not
//! reported; they have no UI rectangle.
//!
//! Space: `rect` is `[x, y, width, height]` in physical pixels of the node's render target, which is
//! the pixel space of `bevy_fairy/screenshot`'s `region`. A node's rect can be passed there as is.

use bevy::prelude::*;
use bevy::text::TextElement;
use bevy::ui::widget::{Text, TextUiReader};
use bevy::ui::{
    CalculatedClip, ComputedNode, ComputedUiRenderTargetInfo, UiGlobalTransform,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::plugin::commands::{add_builtin_command, DebugCommandInfo};
use crate::plugin::ready::FrameTicks;

/// A text colour with alpha at or below this is treated as not drawn. Zero, because a toast that
/// is fading out is still readable until it is fully transparent.
const MIN_ALPHA: f32 = 0.0;

/// The most ancestors named per node.
const MAX_ANCESTORS: usize = 3;

const DEFAULT_LIMIT: usize = 200;

/// A rectangle in physical target pixels, the same shape as `bevy_fairy/screenshot`'s `region`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Params of `bevy_fairy/ui_text`. All fields are optional.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiTextParams {
    /// Case-sensitive substring the node's text must contain.
    pub contains: Option<String>,
    /// Keep nodes whose rect overlaps this region (more than touching an edge).
    pub region: Option<UiRegion>,
    /// Also return nodes that are not visible, each with `hidden_because`. Default `false`.
    #[serde(default)]
    pub include_hidden: bool,
    /// Most nodes returned, applied after sorting top to bottom. Default 200.
    pub limit: Option<usize>,
}

pub(crate) fn register(app: &mut App) {
    add_builtin_command::<UiTextParams, _>(
        app,
        DebugCommandInfo::new(
            "bevy_fairy/ui_text",
            "List the bevy_ui text the player can read: text, on-screen rect, visibility and the names of its nearest ancestors. Hidden nodes are omitted unless include_hidden is true.",
        )
        .params(
            "contains: string, optional, case-sensitive substring; region: {x,y,width,height}, optional, physical target pixels (the space of screenshot's region); include_hidden: bool, default false; limit: integer >= 1, default 200",
        )
        .example(json!({"contains": "Score", "include_hidden": false}))
        .read_only(),
        handle_ui_text,
    );
}

/// Why a node is not readable, in the order the checks run.
#[derive(Clone, Copy)]
enum Hidden {
    EmptyText,
    InheritedVisibility,
    ZeroSize,
    Transparent,
    Clipped,
    OffTarget,
}

impl Hidden {
    fn reason(self) -> &'static str {
        match self {
            Hidden::EmptyText => "empty_text",
            Hidden::InheritedVisibility => "inherited_visibility_false",
            Hidden::ZeroSize => "zero_size (Display::None or collapsed layout)",
            Hidden::Transparent => "text_alpha_zero",
            Hidden::Clipped => "clipped_by_ancestor_overflow",
            Hidden::OffTarget => "outside_render_target",
        }
    }
}

/// The axis-aligned box of `rect` (given by its centre and half-size) after `transform`.
fn aabb(transform: &UiGlobalTransform, half: Vec2) -> Rect {
    let affine = transform.affine();
    let corners = [
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(half.x, half.y),
        Vec2::new(-half.x, half.y),
    ]
    .map(|c| affine.transform_point2(c));
    let mut lo = corners[0];
    let mut hi = corners[0];
    for c in &corners[1..] {
        lo = lo.min(*c);
        hi = hi.max(*c);
    }
    Rect { min: lo, max: hi }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    !a.intersect(b).is_empty()
}

/// Whether the node's box lies entirely outside what an ancestor's `Overflow` lets through.
fn clipped_away(clip: Option<&CalculatedClip>, transform: &UiGlobalTransform, half: Vec2) -> bool {
    let Some(clip) = clip else { return false };
    let Some(rects) = clip.rects() else { return true };
    let affine = transform.affine();
    rects.iter().any(|clip_rect| {
        let corners = [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(half.x, half.y),
            Vec2::new(-half.x, half.y),
        ]
        .map(|c| clip_rect.world_to_clip_local.transform_point2(affine.transform_point2(c)));
        let mut lo = corners[0];
        let mut hi = corners[0];
        for c in &corners[1..] {
            lo = lo.min(*c);
            hi = hi.max(*c);
        }
        !overlaps(Rect { min: lo, max: hi }, clip_rect.rect)
    })
}

struct Row {
    entity: Entity,
    name: Option<String>,
    text: String,
    rect: [i64; 4],
    hidden: Option<Hidden>,
    ancestors: Vec<String>,
    target: (UVec2, f32),
}

#[allow(clippy::type_complexity)]
fn handle_ui_text(
    In(params): In<UiTextParams>,
    mut reader: TextUiReader,
    nodes: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        &ComputedUiRenderTargetInfo,
        Option<&CalculatedClip>,
        Option<&Name>,
    ), With<Text>>,
    names: Query<(Option<&Name>, Option<&ChildOf>)>,
    ticks: Res<FrameTicks>,
) -> Result<Value, String> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 {
        return Err("limit must be >= 1".to_string());
    }
    if params.region.as_ref().is_some_and(|r| r.width == 0 || r.height == 0) {
        return Err("region width and height must both be non-zero".to_string());
    }
    // Summed as f32, not u32: `x + width` past `u32::MAX` panicked a debug build and wrapped to a
    // tiny region in a release build, where an empty answer reads as "no text on screen".
    let region = params.region.as_ref().map(|r| Rect {
        min: Vec2::new(r.x as f32, r.y as f32),
        max: Vec2::new(r.x as f32 + r.width as f32, r.y as f32 + r.height as f32),
    });

    let mut rows: Vec<Row> = Vec::new();
    for (entity, node, transform, visibility, target_info, clip, name) in &nodes {
        let mut text = String::new();
        let mut any_alpha = false;
        for (_, _, element) in reader.iter(entity) {
            if let TextElement::Text { text: piece, color, .. } = element {
                text.push_str(piece);
                if !piece.is_empty() && color.alpha() > MIN_ALPHA {
                    any_alpha = true;
                }
            }
        }
        if params.contains.as_ref().is_some_and(|needle| !text.contains(needle.as_str())) {
            continue;
        }

        let half = node.size / 2.0;
        let target_size = target_info.physical_size();
        let target_rect = Rect {
            min: Vec2::ZERO,
            max: target_size.as_vec2(),
        };
        let bounds = aabb(transform, half);

        let hidden = if text.is_empty() {
            Some(Hidden::EmptyText)
        } else if !visibility.get() {
            Some(Hidden::InheritedVisibility)
        } else if node.size.x <= 0.0 || node.size.y <= 0.0 {
            Some(Hidden::ZeroSize)
        } else if !any_alpha {
            Some(Hidden::Transparent)
        } else if clipped_away(clip, transform, half) {
            Some(Hidden::Clipped)
        } else if !overlaps(bounds, target_rect) {
            Some(Hidden::OffTarget)
        } else {
            None
        };
        if hidden.is_some() && !params.include_hidden {
            continue;
        }

        // A visible node's rect is what the player sees, so it is cut to the target; that is also
        // what makes it a valid screenshot `region`. A hidden node keeps its raw box.
        let shown = if hidden.is_none() { bounds.intersect(target_rect) } else { bounds };
        if region.is_some_and(|region| !overlaps(shown, region)) {
            continue;
        }
        let x0 = shown.min.x.floor() as i64;
        let y0 = shown.min.y.floor() as i64;
        let x1 = shown.max.x.ceil() as i64;
        let y1 = shown.max.y.ceil() as i64;

        let mut ancestors = Vec::new();
        let mut at = names.get(entity).ok().and_then(|(_, parent)| parent.map(ChildOf::parent));
        while let Some(parent) = at {
            if ancestors.len() == MAX_ANCESTORS {
                break;
            }
            let Ok((name, next)) = names.get(parent) else { break };
            if let Some(name) = name {
                ancestors.push(name.as_str().to_string());
            }
            at = next.map(ChildOf::parent);
        }

        rows.push(Row {
            entity,
            name: name.map(|n| n.as_str().to_string()),
            text,
            rect: [x0, y0, x1 - x0, y1 - y0],
            hidden,
            ancestors,
            target: (target_size, target_info.scale_factor()),
        });
    }

    rows.sort_by_key(|r| (r.rect[1], r.rect[0], r.entity));
    let matched = rows.len();
    let truncated = matched > limit;
    rows.truncate(limit);

    // Targets differ only with several cameras or windows. Then no single size describes the
    // reply, so say so and put the size on each node instead of reporting one that is wrong.
    let single = rows.windows(2).all(|w| w[0].target == w[1].target);
    let (target_size, scale_factor) = match rows.first() {
        Some(first) if single => (json!([first.target.0.x, first.target.0.y]), json!(first.target.1)),
        _ => (Value::Null, Value::Null),
    };

    let nodes: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let mut node = json!({
                "entity": r.entity.to_bits(),
                "name": r.name,
                "text": r.text,
                "rect": r.rect,
                "visible": r.hidden.is_none(),
                "ancestors": r.ancestors,
            });
            if let Some(why) = r.hidden {
                node["hidden_because"] = json!(why.reason());
            }
            if !single {
                node["target_size"] = json!([r.target.0.x, r.target.0.y]);
                node["scale_factor"] = json!(r.target.1);
            }
            node
        })
        .collect();

    let mut reply = json!({
        "frame": ticks.0,
        "target_size": target_size,
        "scale_factor": scale_factor,
        "matched": matched,
        "truncated": truncated,
        "nodes": nodes,
    });
    if !single {
        reply["target_size_unavailable"] =
            json!("nodes render to different targets; see each node's target_size");
    }
    if truncated {
        reply["narrow_with"] = json!("pass contains or region to narrow, or raise limit");
    }
    Ok(reply)
}
