//! Screenshot capture, from an **offscreen render target** or from the **window**, with region
//! cropping and zoom. The PNG comes back over BRP, so the agent needs no shared filesystem.
//!
//! # Two sources
//!
//! - `offscreen` (the default when a host inserts [`DebugCaptureTarget`]): captures an `Image` that a
//!   camera renders to. It is pure Bevy: no OS screen capture, no window manager, no focus change. It
//!   works while the game is buried behind other windows, on another Space, or minimised. It does not
//!   include UI drawn to the window, so a host that wants UI in the picture uses `window`.
//! - `window` (the default otherwise): `Screenshot::primary_window()` reads the game's own swapchain.
//!   It does not raise or focus the window. The OS keeps a window's surface current only while the
//!   window is on screen, so a covered or minimised window may give a stale frame, and on macOS an
//!   unfocused window has been measured to give one flat colour. Use `offscreen` for those cases.
//!
//! Asking for `window` when no window exists is an error. There is no silent fallback between the two,
//! so the picture always comes from the source the caller named or the one the host set up.
//!
//! # The host supplies the target
//!
//! A library cannot know which view you want captured, so it does not guess: insert
//! [`DebugCaptureTarget`] with a handle to an `Image` that one of your cameras renders to
//! (`RenderTarget::Image`). Give that camera whatever marker your other camera queries exclude, or,
//! better, filter those positively on your own main-camera marker, which is immune to any number of
//! extra cameras.
//!
//! # Captures come back over BRP
//!
//! `bevy_fairy/screenshot` returns at once with an id. The frame is read a few frames later and
//! kept in [`Captures`], which holds the last eight. `bevy_fairy/capture` returns the PNG as base64
//! once it is ready. Ostrowski & Aroudj (GSTF Journal of Computing 3:10, 2013, DOI
//! 10.7603/s40601-013-0010-4) return regression-test artefacts over the same connection that drives
//! the game, for the same reason: the machine that runs the test may not be the one that reads it.

use std::collections::VecDeque;
use std::io::Cursor;

use bevy::prelude::*;
use bevy::remote::{error_codes, BrpError, BrpResult};
use bevy::render::view::window::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;
use image::{imageops::FilterType, ImageFormat};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::plugin::ready::FrameTicks;

/// How many finished or pending captures [`Captures`] keeps. The oldest is dropped.
const KEPT: usize = 8;

/// The offscreen image `bevy_fairy/screenshot` captures.
///
/// Insert this with a handle to an `Image` that one of your cameras renders to. Nothing here spawns a
/// camera: which view is worth capturing, at what resolution, and how it tracks your gameplay camera
/// are all decisions only the host can make.
#[derive(Resource, Clone)]
pub struct DebugCaptureTarget {
    pub image: Handle<Image>,
}

/// Parameters for the `bevy_fairy/screenshot` BRP method.
#[derive(Debug, Deserialize)]
pub struct ScreenshotParams {
    /// Path to also write the PNG to, on the game's machine. Optional: the PNG is always returned
    /// through `bevy_fairy/capture`, and a file is written only when this is given.
    pub path: Option<String>,
    /// Region to crop, in physical pixels: `{ x, y, width, height }`. Clamped to the captured image;
    /// a region entirely outside it is an error rather than an empty file.
    pub region: Option<Region>,
    /// Scale factor applied after cropping. `1.0` leaves the image alone. The scaled image is at
    /// most [`MAX_ZOOMED_PIXELS`] pixels; a `zoom` that would make it larger is refused.
    #[serde(default = "one")]
    pub zoom: f32,
    /// `"offscreen"` or `"window"`. Defaults to offscreen when a [`DebugCaptureTarget`] exists,
    /// otherwise window.
    pub source: Option<String>,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where a capture stands.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureState {
    Pending,
    Done { png: Vec<u8>, width: u32, height: u32, provenance: Provenance },
    Failed(String),
}

/// Where a finished capture came from and when, so a caller can tell a fresh frame from a stale one.
///
/// Frames are the plugin's own count (the `frame` of `bevy_fairy/status`): `frame_requested` is the
/// frame the request was handled on, `frame_captured` the frame its pixels were delivered on. The
/// pixels are the render of some frame between the two, never later than `frame_captured`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    /// `"offscreen"` or `"window"`: the source that was actually read.
    pub source: &'static str,
    pub frame_requested: u32,
    pub frame_captured: u32,
}

/// The last few captures, by id. A capture is `Pending` from the request until its frame is read.
#[derive(Resource, Default)]
pub struct Captures {
    next_id: u64,
    entries: VecDeque<(u64, CaptureState)>,
}

impl Captures {
    /// Starts a capture and returns its id. Drops the oldest entry once more than [`KEPT`] exist.
    pub(crate) fn begin(&mut self) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        self.entries.push_back((id, CaptureState::Pending));
        while self.entries.len() > KEPT {
            self.entries.pop_front();
        }
        id
    }

    /// Records the outcome of capture `id`. Ignored when that capture has already been dropped.
    pub(crate) fn finish(&mut self, id: u64, state: CaptureState) {
        if let Some(slot) = self.entries.iter_mut().find(|(i, _)| *i == id) {
            slot.1 = state;
        }
    }

    /// The state of capture `id`, or `None` when it was never made or has been dropped.
    pub(crate) fn get(&self, id: u64) -> Option<&CaptureState> {
        self.entries.iter().find(|(i, _)| *i == id).map(|(_, s)| s)
    }
}

#[derive(Clone, Copy)]
enum Source {
    Offscreen,
    Window,
}

/// Build a `BrpError` for a malformed parameter payload.
fn invalid_params(message: String) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message, data: None }
}

/// BRP handler: `bevy_fairy/screenshot`.
pub fn handle_screenshot(
    In(params): In<Option<Value>>,
    mut commands: Commands,
    target: Option<Res<DebugCaptureTarget>>,
    windows: Query<(), With<PrimaryWindow>>,
    mut captures: ResMut<Captures>,
    ticks: Res<FrameTicks>,
) -> BrpResult {
    let params: ScreenshotParams = match params {
        Some(p) => serde_json::from_value(p)
            .map_err(|e| invalid_params(format!("invalid screenshot params: {e}")))?,
        None => ScreenshotParams { path: None, region: None, zoom: 1.0, source: None },
    };

    let source = match params.source.as_deref() {
        None if target.is_some() => Source::Offscreen,
        None => Source::Window,
        Some("offscreen") => Source::Offscreen,
        Some("window") => Source::Window,
        Some(_) => return Err(invalid_params("source must be offscreen or window".to_string())),
    };

    if !params.zoom.is_finite() || params.zoom <= 0.0 {
        return Err(invalid_params(format!(
            "zoom must be finite and greater than zero, got {}",
            params.zoom
        )));
    }
    // Written as a nested `if` rather than a let-chain: this crate is edition 2021, where let-chains
    // are not available regardless of compiler version.
    if let Some(r) = &params.region {
        if r.width == 0 || r.height == 0 {
            return Err(invalid_params(
                "region width and height must both be non-zero".to_string(),
            ));
        }
    }

    let screenshot = match source {
        // Loudly, not with a window fallback: the host chose the view it wants captured.
        Source::Offscreen => match target {
            Some(target) => Screenshot::image(target.image.clone()),
            None => {
                return Err(BrpError {
                    code: error_codes::INTERNAL_ERROR,
                    message: "no DebugCaptureTarget resource: the host must insert one with an Image a \
                              camera renders to, or ask for source \"window\""
                        .to_string(),
                    data: None,
                })
            }
        },
        Source::Window => {
            if windows.is_empty() {
                return Err(BrpError {
                    code: error_codes::INTERNAL_ERROR,
                    message: "no primary window to capture".to_string(),
                    data: None,
                });
            }
            Screenshot::primary_window()
        }
    };

    let id = captures.begin();
    let source_name = match source {
        Source::Offscreen => "offscreen",
        Source::Window => "window",
    };
    let frame_requested = ticks.0;
    let path = params.path;
    let reported = path.clone();
    let region = params.region;
    let zoom = params.zoom;

    commands.spawn(screenshot).observe(move |trigger: On<ScreenshotCaptured>, mut captures: ResMut<Captures>, ticks: Res<FrameTicks>| {
        let provenance = Provenance { source: source_name, frame_requested, frame_captured: ticks.0 };
        let state = match encode_capture(&trigger.image, region.as_ref(), zoom) {
            Ok((png, width, height)) => match path.as_deref() {
                Some(p) => match std::fs::write(p, &png) {
                    Ok(()) => {
                        info!("bevy_fairy/screenshot: wrote {p}");
                        CaptureState::Done { png, width, height, provenance }
                    }
                    Err(e) => CaptureState::Failed(format!("could not write {p}: {e}")),
                },
                None => CaptureState::Done { png, width, height, provenance },
            },
            Err(e) => CaptureState::Failed(e),
        };
        if let CaptureState::Failed(why) = &state {
            error!("bevy_fairy/screenshot: capture {id} failed: {why}");
        }
        captures.finish(id, state);
    });

    Ok(json!({ "id": id, "path": reported }))
}

/// Parameters for the `bevy_fairy/capture` BRP method.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureParams {
    pub id: u64,
}

/// BRP handler: `bevy_fairy/capture`. Returns the state of one capture, and its PNG once done.
pub(crate) fn handle_capture(In(params): In<Option<Value>>, captures: Res<Captures>) -> BrpResult {
    let params: CaptureParams = match params {
        Some(p) => serde_json::from_value(p).map_err(|e| {
            invalid_params(format!("bevy_fairy/capture: {e}; example: {}", json!({"id": 1})))
        })?,
        None => return Err(invalid_params("bevy_fairy/capture needs {\"id\": n}".to_string())),
    };
    match captures.get(params.id) {
        None => Err(invalid_params(format!(
            "no capture {} (only the last {KEPT} are kept)",
            params.id
        ))),
        Some(CaptureState::Pending) => Ok(json!({ "status": "pending" })),
        Some(CaptureState::Done { png, width, height, provenance }) => Ok(json!({
            "status": "done",
            "width": width,
            "height": height,
            "source": provenance.source,
            "frame_requested": provenance.frame_requested,
            "frame_captured": provenance.frame_captured,
            "png_base64": base64_encode(png),
        })),
        Some(CaptureState::Failed(error)) => Ok(json!({ "status": "failed", "error": error })),
    }
}

/// The most pixels `zoom` may make of a capture: 4096 x 4096, twice a 4K frame.
const MAX_ZOOMED_PIXELS: f64 = 16_777_216.0;

/// Convert, crop, scale and encode to PNG. Returns the PNG bytes and the final size. Split out so
/// every failure has one place to be reported from, and so the observer closure stays readable.
fn encode_capture(
    image: &Image,
    region: Option<&Region>,
    zoom: f32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let dynamic = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| format!("the captured image could not be converted: {e}"))?;

    // Drop the alpha channel. With HDR enabled it carries brightness rather than opacity, so keeping
    // it makes the PNG look wrong; `bevy_render`'s own `save_to_disk` does exactly this.
    let mut rgb = dynamic.to_rgb8();

    if let Some(r) = region {
        let (w, h) = (rgb.width(), rgb.height());
        if r.x >= w || r.y >= h {
            return Err(format!(
                "region origin ({}, {}) is outside the {w}x{h} capture",
                r.x, r.y
            ));
        }
        // Clamp rather than refuse: a region running off the edge is a reasonable request, an origin
        // off the edge is a mistake.
        let width = r.width.min(w - r.x);
        let height = r.height.min(h - r.y);
        rgb = image::imageops::crop_imm(&rgb, r.x, r.y, width, height).to_image();
    }

    if zoom != 1.0 {
        let width = (f64::from(rgb.width()) * f64::from(zoom)).round().max(1.0);
        let height = (f64::from(rgb.height()) * f64::from(zoom)).round().max(1.0);
        // Refused, not attempted: the resize allocates the whole image inside the game's own
        // observer, and `{"zoom": 1e9}` used to abort the game on a 256 GB allocation.
        if width * height > MAX_ZOOMED_PIXELS {
            return Err(format!(
                "zoom {zoom} would make the {}x{} capture a {width:.0}x{height:.0} image; `zoom` makes at \
                 most {MAX_ZOOMED_PIXELS:.0} pixels (4096 x 4096). Pass a smaller `zoom`, or a `region` to \
                 zoom into",
                rgb.width(),
                rgb.height()
            ));
        }
        // Lanczos3 for downscales, which is what a zoom < 1 mostly is; it is also acceptable
        // magnifying, and one filter beats a branch nobody tuned.
        rgb = image::imageops::resize(&rgb, width as u32, height as u32, FilterType::Lanczos3);
    }

    let (width, height) = (rgb.width(), rgb.height());
    let mut buf = Cursor::new(Vec::new());
    rgb.write_to(&mut buf, ImageFormat::Png)
        .map_err(|e| format!("could not encode the PNG: {e}"))?;
    Ok((buf.into_inner(), width, height))
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// RFC 4648 base64, standard alphabet, `=` padding. Written out here so the crate takes no encoding
/// dependency.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(BASE64[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        let cases: [(&str, &str); 7] = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in cases {
            assert_eq!(base64_encode(input.as_bytes()), expected, "input {input:?}");
        }
    }

    #[test]
    fn a_done_capture_reports_where_and_when_it_was_taken() {
        let mut world = World::new();
        let mut captures = Captures::default();
        let id = captures.begin();
        let provenance = Provenance { source: "window", frame_requested: 40, frame_captured: 43 };
        captures.finish(id, CaptureState::Done { png: vec![1, 2, 3], width: 4, height: 5, provenance });
        world.insert_resource(captures);
        let reply = world
            .run_system_once_with(handle_capture, Some(json!({ "id": id })))
            .expect("system runs")
            .expect("capture answers");
        assert_eq!(reply["status"], "done");
        assert_eq!(reply["source"], "window");
        assert_eq!(reply["frame_requested"], 40);
        assert_eq!(reply["frame_captured"], 43);
    }

    #[test]
    fn captures_keep_the_last_eight() {
        let mut captures = Captures::default();
        let ids: Vec<u64> = (0..10).map(|_| captures.begin()).collect();
        assert!(captures.get(ids[0]).is_none());
        assert!(captures.get(ids[1]).is_none());
        assert_eq!(captures.get(ids[2]), Some(&CaptureState::Pending));
        captures.finish(ids[9], CaptureState::Failed("boom".into()));
        assert_eq!(captures.get(ids[9]), Some(&CaptureState::Failed("boom".into())));
    }

    fn black(width: u32, height: u32) -> Image {
        Image::new_target_texture(width, height, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None)
    }

    /// `zoom` had no upper bound: `{"zoom": 1e9}` asked `image::imageops::resize` for a 4e9 x 4e9
    /// image inside the game's observer, which panics or aborts the whole game on the allocation.
    /// A request this server cannot honour is a failed capture that says why, never an attempt.
    #[test]
    fn a_zoom_that_would_make_a_huge_image_is_refused_before_it_is_allocated() {
        // Absurd values reach the refusal and not an overflow or an allocation.
        encode_capture(&black(4, 4), None, 1e9).expect_err("1e9 is refused");
        encode_capture(&black(4, 4), None, f32::MAX).expect_err("f32::MAX is refused");
        let refusal = encode_capture(&black(4, 4), None, 2500.0).expect_err("a 10000 x 10000 image is refused");
        assert!(refusal.contains("zoom") && refusal.contains("10000x10000"), "{refusal}");
        assert!(refusal.contains("region"), "it says how to get the detail another way: {refusal}");
    }

    #[test]
    fn a_zoom_that_fits_still_scales_the_cropped_region() {
        let (_, width, height) = encode_capture(&black(4, 4), None, 8.0).expect("8x of 4x4");
        assert_eq!((width, height), (32, 32));
        let (_, width, height) = encode_capture(&black(4, 4), None, 0.5).expect("0.5x of 4x4");
        assert_eq!((width, height), (2, 2));
        let region = Region { x: 0, y: 0, width: 2, height: 1 };
        let (_, width, height) = encode_capture(&black(4, 4), Some(&region), 16.0).expect("16x of a 2x1 crop");
        assert_eq!((width, height), (32, 16));
    }
}
