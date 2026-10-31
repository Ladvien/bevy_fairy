//! How each value inside a watched result moved, not only the result as a whole.
//!
//! `watch` samples one BRP result over time. A result with several fields -- a component with
//! an angle and a frame counter -- differs from its first sample as soon as any field does, so a
//! whole-result summary calls a spinner whose angle froze but whose counter still runs
//! "changing", which is the opposite of the answer an agent was asking for ("which of its values
//! stopped?"). This module gives every value its own summary, with the same four numbers the
//! whole result gets.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// Most fields a reply lists. A result is read in full, so the number a caller can narrow with
/// `pointer` is the number `fields_omitted` reports; the cut is only on what is printed.
pub const MAX_FIELDS: usize = 40;

/// How one value moved across a series of samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movement {
    /// Whether any sample differs from the first.
    pub changed: bool,
    /// How many different values (an absent one counts as a value) the series held.
    pub distinct_values: usize,
    /// The frame of the first sample that differs from the first sample.
    pub first_change_frame: Option<u64>,
    /// The frame of the first sample of the final run of equal values, when that run holds at
    /// least two samples. `None` when the last sample is the only one holding its value: nothing
    /// shows the value stayed.
    pub constant_since_frame: Option<u64>,
}

impl Movement {
    fn to_json(&self, pointer: &str) -> Value {
        json!({
            "pointer": pointer,
            "changed": self.changed,
            "distinct_values": self.distinct_values,
            "first_change_frame": self.first_change_frame,
            "constant_since_frame": self.constant_since_frame,
        })
    }
}

/// `values[i]` is what sample `i` held, read on a frame starting at `frames[i]`; `None` is a
/// value that sample did not have. Both slices are the same non-zero length.
pub fn movement(frames: &[u64], values: &[Option<&Value>]) -> Movement {
    debug_assert_eq!(frames.len(), values.len());
    let first = values[0];
    let last = values[values.len() - 1];
    let first_change_frame = values.iter().position(|v| *v != first).map(|i| frames[i]);
    let mut distinct: Vec<Option<&Value>> = Vec::new();
    for value in values {
        if !distinct.contains(value) {
            distinct.push(*value);
        }
    }
    let run_start = values.iter().rposition(|v| *v != last).map_or(0, |i| i + 1);
    let constant_since_frame = (run_start + 1 < values.len()).then(|| frames[run_start]);
    Movement {
        changed: first_change_frame.is_some(),
        distinct_values: distinct.len(),
        first_change_frame,
        constant_since_frame,
    }
}

/// The per-field summaries of a series, ready to put in a reply.
#[derive(Debug)]
pub struct Fields {
    /// How many fields the samples held between them.
    pub total: usize,
    /// The summaries printed: fields that changed first, then those that did not, each group in
    /// pointer order, cut at [`MAX_FIELDS`].
    pub listed: Vec<(String, Movement)>,
}

impl Fields {
    /// The summaries as the `fields` array.
    pub fn to_json(&self) -> Value {
        Value::Array(self.listed.iter().map(|(pointer, m)| m.to_json(pointer)).collect())
    }

    /// How many fields were left out of `listed`.
    pub fn omitted(&self) -> usize {
        self.total - self.listed.len()
    }
}

/// Summarise every field of `samples` (the sampled values, `frames[i]` for `samples[i]`).
///
/// A field is a leaf of the JSON: a number, string, boolean or null, or an empty object or
/// array. Its `pointer` is `prefix` followed by the RFC 6901 path from the sampled value, so a
/// caller can hand it back to `watch` as `pointer` when `prefix` is the pointer the sampled value
/// was cut with. An array position is a position, not an identity: a despawn that shifts the rows
/// of a `world.query` shifts every field after it.
///
/// `None` when the samples hold fewer than two fields: a lone value's summary is the whole
/// result's, which the reply already carries.
pub fn fields(frames: &[u64], samples: &[&Value], prefix: &str) -> Option<Fields> {
    let mut by_pointer: BTreeMap<String, Vec<Option<&Value>>> = BTreeMap::new();
    for (i, sample) in samples.iter().copied().enumerate() {
        collect_leaves(sample, prefix.to_string(), &mut |pointer, leaf| {
            by_pointer.entry(pointer).or_insert_with(|| vec![None; samples.len()])[i] = Some(leaf);
        });
    }
    if by_pointer.len() < 2 {
        return None;
    }
    let total = by_pointer.len();
    let mut all: Vec<(String, Movement)> =
        by_pointer.into_iter().map(|(pointer, values)| (pointer, movement(frames, &values))).collect();
    // Stable, so each group stays in pointer order.
    all.sort_by_key(|(_, m)| !m.changed);
    all.truncate(MAX_FIELDS);
    Some(Fields { total, listed: all })
}

fn collect_leaves<'a>(value: &'a Value, pointer: String, found: &mut impl FnMut(String, &'a Value)) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                collect_leaves(child, format!("{pointer}/{}", escape(key)), found);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                collect_leaves(child, format!("{pointer}/{index}"), found);
            }
        }
        leaf => found(pointer, leaf),
    }
}

/// An object key as one RFC 6901 reference token.
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[Value]) -> (Vec<u64>, Vec<&Value>) {
        // Frames 100, 101, ... one per sample.
        ((100..).take(values.len()).collect(), values.iter().collect())
    }

    fn listed(fields: &Fields, pointer: &str) -> Movement {
        fields
            .listed
            .iter()
            .find(|(p, _)| p == pointer)
            .unwrap_or_else(|| panic!("{pointer} is not listed in {:?}", fields.listed))
            .1
            .clone()
    }

    #[test]
    fn a_value_that_froze_says_the_frame_its_last_run_began() {
        let values: Vec<Value> = [1, 1, 2, 2, 2].iter().map(|n| json!(n)).collect();
        let (frames, refs) = series(&values);
        let refs: Vec<Option<&Value>> = refs.into_iter().map(Some).collect();
        assert_eq!(
            movement(&frames, &refs),
            Movement { changed: true, distinct_values: 2, first_change_frame: Some(102), constant_since_frame: Some(102) }
        );
    }

    #[test]
    fn a_value_still_moving_at_the_last_sample_is_not_constant_since_anything() {
        let values: Vec<Value> = [1, 2, 3].iter().map(|n| json!(n)).collect();
        let (frames, refs) = series(&values);
        let refs: Vec<Option<&Value>> = refs.into_iter().map(Some).collect();
        let m = movement(&frames, &refs);
        assert_eq!(m.constant_since_frame, None);
        assert_eq!(m.distinct_values, 3);
    }

    #[test]
    fn a_value_that_never_moved_is_constant_from_the_first_sample() {
        let values: Vec<Value> = [5, 5, 5].iter().map(|n| json!(n)).collect();
        let (frames, refs) = series(&values);
        let refs: Vec<Option<&Value>> = refs.into_iter().map(Some).collect();
        assert_eq!(
            movement(&frames, &refs),
            Movement { changed: false, distinct_values: 1, first_change_frame: None, constant_since_frame: Some(100) }
        );
    }

    /// The case the module exists for: one field freezes while its neighbour keeps counting. The
    /// whole value changes at every sample, and only the per-field summary says which one stopped.
    #[test]
    fn one_field_frozen_beside_one_still_counting_is_told_apart() {
        let values: Vec<Value> = (0..6).map(|i| json!({"angle": f64::from(i.min(3)), "frames": i})).collect();
        let (frames, refs) = series(&values);
        let whole: Vec<Option<&Value>> = refs.iter().copied().map(Some).collect();
        assert_eq!(movement(&frames, &whole).constant_since_frame, None, "the whole value never settles");

        let fields = fields(&frames, &refs, "/0/components/fixture_game::Spinner").expect("two fields");
        assert_eq!(fields.total, 2);
        let angle = listed(&fields, "/0/components/fixture_game::Spinner/angle");
        assert!(angle.changed);
        assert_eq!(angle.first_change_frame, Some(101));
        assert_eq!(angle.constant_since_frame, Some(103), "angle holds 3.0 from sample 3 on");
        let counter = listed(&fields, "/0/components/fixture_game::Spinner/frames");
        assert_eq!(counter.constant_since_frame, None);
        assert_eq!(counter.distinct_values, 6);
    }

    #[test]
    fn fields_that_changed_come_before_fields_that_did_not() {
        let values: Vec<Value> = (0..3).map(|i| json!({"a": 1, "b": i, "c": 1, "d": i})).collect();
        let (frames, refs) = series(&values);
        let fields = fields(&frames, &refs, "").expect("fields");
        let order: Vec<&str> = fields.listed.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(order, ["/b", "/d", "/a", "/c"]);
    }

    #[test]
    fn a_field_missing_from_a_sample_counts_as_a_change_not_as_null() {
        let values = [json!({"x": 1, "y": 7}), json!({"x": 1}), json!({"x": 1, "y": null})];
        let (frames, refs) = series(&values);
        let fields = fields(&frames, &refs, "").expect("fields");
        let y = listed(&fields, "/y");
        assert!(y.changed);
        assert_eq!(y.distinct_values, 3, "7, absent and null are three different states");
        assert!(!listed(&fields, "/x").changed);
    }

    #[test]
    fn a_key_with_a_slash_or_tilde_is_escaped_so_the_pointer_resolves() {
        let values = [json!({"a/b": 1, "c~d": 1}), json!({"a/b": 2, "c~d": 2})];
        let (frames, refs) = series(&values);
        let fields = fields(&frames, &refs, "/p").expect("fields");
        for (pointer, _) in &fields.listed {
            let resolved = json!({"p": values[0].clone()});
            assert!(resolved.pointer(pointer).is_some(), "{pointer} does not resolve");
        }
        assert_eq!(fields.listed[0].0, "/p/a~1b");
        assert_eq!(fields.listed[1].0, "/p/c~0d");
    }

    #[test]
    fn arrays_are_walked_by_position_and_empty_containers_are_leaves() {
        let values = [json!([{"v": [1, 2]}, {}]), json!([{"v": [1, 3]}, {}])];
        let (frames, refs) = series(&values);
        let fields = fields(&frames, &refs, "").expect("fields");
        assert_eq!(fields.total, 3);
        assert!(listed(&fields, "/0/v/1").changed);
        assert!(!listed(&fields, "/0/v/0").changed);
        assert!(!listed(&fields, "/1").changed, "an empty object is one field");
    }

    #[test]
    fn a_lone_value_has_no_breakdown_because_it_is_the_whole_result() {
        let scalars = [json!(1.5), json!(2.5)];
        let (frames, refs) = series(&scalars);
        assert!(fields(&frames, &refs, "").is_none());
        let one_field = [json!({"only": 1}), json!({"only": 2})];
        let (frames, refs) = series(&one_field);
        assert!(fields(&frames, &refs, "").is_none());
    }

    #[test]
    fn a_long_list_is_cut_with_the_changing_fields_kept_and_the_cut_counted() {
        let values: Vec<Value> = (0..3)
            .map(|i| {
                let mut map = serde_json::Map::new();
                for n in 0..100 {
                    // Only fields 97..100 move; the cut must not drop them for being last.
                    map.insert(format!("f{n:03}"), json!(if n >= 97 { i } else { 0 }));
                }
                Value::Object(map)
            })
            .collect();
        let (frames, refs) = series(&values);
        let fields = fields(&frames, &refs, "").expect("fields");
        assert_eq!(fields.total, 100);
        assert_eq!(fields.listed.len(), MAX_FIELDS);
        assert_eq!(fields.omitted(), 100 - MAX_FIELDS);
        assert!(fields.listed[..3].iter().all(|(_, m)| m.changed));
        assert!(fields.listed[3..].iter().all(|(_, m)| !m.changed));
    }
}
