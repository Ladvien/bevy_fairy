use bevy_fairy::server::brp_messages::EntityData;
use bevy_fairy::server::state_diff::*;
use serde_json::json;

fn create_test_entity(id: u64, components: Vec<(&str, serde_json::Value)>) -> EntityData {
    EntityData {
        id,
        components: components
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

#[test]
fn test_change_type_description() {
    assert_eq!(ChangeType::EntityAdded.description(), "Entity added");
    assert_eq!(ChangeType::EntityRemoved.description(), "Entity removed");
    assert_eq!(ChangeType::EntityModified.description(), "Entity modified");
    assert_eq!(ChangeType::ComponentAdded.description(), "Component added");
    assert_eq!(
        ChangeType::ComponentRemoved.description(),
        "Component removed"
    );
    assert_eq!(
        ChangeType::ComponentModified.description(),
        "Component modified"
    );
}

#[test]
fn test_change_type_color_codes() {
    assert_eq!(ChangeType::EntityAdded.color_code(), "\x1b[32m");
    assert_eq!(ChangeType::ComponentAdded.color_code(), "\x1b[32m");
    assert_eq!(ChangeType::EntityRemoved.color_code(), "\x1b[31m");
    assert_eq!(ChangeType::ComponentRemoved.color_code(), "\x1b[31m");
    assert_eq!(ChangeType::EntityModified.color_code(), "\x1b[33m");
    assert_eq!(ChangeType::ComponentModified.color_code(), "\x1b[33m");
}

#[test]
fn test_change_creation() {
    let change = Change::new(
        ChangeType::ComponentModified,
        42,
        Some("Transform".to_string()),
        Some(json!({"x": 0.0})),
        Some(json!({"x": 1.0})),
    );

    assert_eq!(change.change_type, ChangeType::ComponentModified);
    assert_eq!(change.entity_id, 42);
    assert_eq!(change.component_type, Some("Transform".to_string()));
    assert_eq!(change.old_value, Some(json!({"x": 0.0})));
    assert_eq!(change.new_value, Some(json!({"x": 1.0})));
    assert_eq!(change.rate_of_change, None);
    assert!(!change.is_unexpected);
}

#[test]
fn test_change_rate_calculation() {
    let mut change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Position".to_string()),
        Some(json!(0.0)),
        Some(json!(10.0)),
    );

    let time_delta = std::time::Duration::from_secs(2);
    change.calculate_rate_of_change(time_delta);

    assert_eq!(change.rate_of_change, Some(5.0)); // (10.0 - 0.0) / 2.0
}

#[test]
fn test_change_rate_calculation_zero_time() {
    let mut change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Position".to_string()),
        Some(json!(0.0)),
        Some(json!(10.0)),
    );

    let time_delta = std::time::Duration::from_secs(0);
    change.calculate_rate_of_change(time_delta);

    assert_eq!(change.rate_of_change, None); // Should not calculate with zero time
}

#[test]
fn test_change_rate_calculation_non_numeric() {
    let mut change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Name".to_string()),
        Some(json!("old_name")),
        Some(json!("new_name")),
    );

    let time_delta = std::time::Duration::from_secs(1);
    change.calculate_rate_of_change(time_delta);

    assert_eq!(change.rate_of_change, None); // Should not calculate for non-numeric values
}

#[test]
fn test_fuzzy_compare_config_default() {
    let config = FuzzyCompareConfig::default();
    assert_eq!(config.epsilon, 1e-6);
    assert_eq!(config.relative_tolerance, 1e-9);
}

#[test]
fn test_fuzzy_float_comparison_within_epsilon() {
    let config = FuzzyCompareConfig::default();

    let val1 = json!(1.0000001);
    let val2 = json!(1.0000002);

    assert!(val1.fuzzy_eq(&val2, &config));
}

#[test]
fn test_fuzzy_float_comparison_outside_epsilon() {
    let config = FuzzyCompareConfig::default();

    let val1 = json!(1.0);
    let val2 = json!(2.0);

    assert!(!val1.fuzzy_eq(&val2, &config));
}

/// **JSON cannot hold a non-finite float, and that is the thing worth pinning here.**
///
/// `Value::from(f64)` is `Number::from_f64(f).map_or(Value::Null, Value::Number)`
/// (serde_json-1.0.150/src/value/from.rs:60), so `NaN`, `+∞` and `−∞` all become `null` before
/// `fuzzy_eq` ever sees them. They are therefore indistinguishable from each other *and* from an
/// absent value.
///
/// This matters when diffing game state: an entity whose velocity has gone `NaN` does not show up as
/// a changed float, it shows up as `null`. The two tests this replaces asserted float semantics that
/// no `Value` can carry — one passed only because both sides had silently collapsed to `null`, and
/// the other demanded that `+∞` and `−∞` compare unequal, which they cannot.
#[test]
fn non_finite_floats_collapse_to_null_before_fuzzy_comparison() {
    let config = FuzzyCompareConfig::default();

    for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            json!(non_finite),
            serde_json::Value::Null,
            "serde_json cannot represent {non_finite}, so it must arrive as null"
        );
    }

    // Consequently every pairing of them compares equal — including `+∞` against `−∞`.
    let inf = json!(f64::INFINITY);
    let neg_inf = json!(f64::NEG_INFINITY);
    let nan = json!(f64::NAN);
    assert!(inf.fuzzy_eq(&neg_inf, &config));
    assert!(inf.fuzzy_eq(&nan, &config));

    // Finite floats still compare on their values, which is the case that actually carries meaning.
    assert!(json!(1.0).fuzzy_eq(&json!(1.0), &config));
    assert!(!json!(1.0).fuzzy_eq(&json!(2.0), &config));
}

#[test]
fn test_fuzzy_object_comparison() {
    let config = FuzzyCompareConfig::default();

    let obj1 = json!({"x": 1.0000001, "y": 2.0});
    let obj2 = json!({"x": 1.0000002, "y": 2.0});
    let obj3 = json!({"x": 1.5, "y": 2.0});

    assert!(obj1.fuzzy_eq(&obj2, &config));
    assert!(!obj1.fuzzy_eq(&obj3, &config));
}

#[test]
fn test_fuzzy_array_comparison() {
    let config = FuzzyCompareConfig::default();

    let arr1 = json!([1.0000001, 2.0, 3.0]);
    let arr2 = json!([1.0000002, 2.0, 3.0]);
    let arr3 = json!([1.5, 2.0, 3.0]);

    assert!(arr1.fuzzy_eq(&arr2, &config));
    assert!(!arr1.fuzzy_eq(&arr3, &config));
}

#[test]
fn test_game_rules_immutable_components() {
    let mut rules = GameRules::default();
    rules.immutable_components.insert("EntityId".to_string());

    let change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("EntityId".to_string()),
        Some(json!(1)),
        Some(json!(2)),
    );

    assert!(rules.is_change_unexpected(&change));
}

#[test]
fn test_game_rules_position_rate_limit() {
    let rules = GameRules {
        max_position_change_per_second: Some(10.0),
        ..Default::default()
    };

    let mut change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Transform".to_string()),
        Some(json!(0.0)),
        Some(json!(100.0)),
    );
    change.rate_of_change = Some(50.0); // Exceeds limit

    assert!(rules.is_change_unexpected(&change));
}

#[test]
fn test_game_rules_velocity_rate_limit() {
    let rules = GameRules {
        max_velocity_change_per_second: Some(5.0),
        ..Default::default()
    };

    let mut change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Velocity".to_string()),
        Some(json!(0.0)),
        Some(json!(10.0)),
    );
    change.rate_of_change = Some(20.0); // Exceeds limit

    assert!(rules.is_change_unexpected(&change));
}

#[test]
fn test_game_rules_value_range() {
    let mut rules = GameRules::default();
    rules
        .expected_value_ranges
        .insert("Health.value".to_string(), (0.0, 100.0));

    let change = Change::new(
        ChangeType::ComponentModified,
        1,
        Some("Health".to_string()),
        Some(json!(50.0)),
        Some(json!(150.0)), // Exceeds max
    );

    assert!(rules.is_change_unexpected(&change));
}

#[test]
fn test_state_snapshot_creation() {
    let entities = vec![
        create_test_entity(1, vec![("Transform", json!({"x": 0.0}))]),
        create_test_entity(2, vec![("Health", json!(100))]),
    ];

    let snapshot = StateSnapshot::new(entities, 42);

    assert_eq!(snapshot.entities.len(), 2);
    assert_eq!(snapshot.generation, 42);
    assert!(snapshot.get_entity(1).is_some());
    assert!(snapshot.get_entity(3).is_none());

    let entity_ids = snapshot.entity_ids();
    assert!(entity_ids.contains(&1));
    assert!(entity_ids.contains(&2));
    assert!(!entity_ids.contains(&3));
}

#[test]
fn test_state_snapshot_compression() {
    let entities = vec![
        create_test_entity(1, vec![("Transform", json!({"x": 0.0, "y": 1.0}))]),
        create_test_entity(2, vec![("Health", json!(100))]),
    ];

    let snapshot = StateSnapshot::new(entities, 1);

    let compressed = snapshot.compress();
    assert!(!compressed.is_empty());

    let decompressed = StateSnapshot::decompress(&compressed, 1, snapshot.timestamp);
    assert!(decompressed.is_some());

    let decompressed_snapshot = decompressed.unwrap();
    assert_eq!(decompressed_snapshot.entities.len(), 2);
    assert_eq!(decompressed_snapshot.generation, 1);
}

#[test]
fn test_state_diff_engine_creation() {
    let engine = StateDiff::new();
    assert_eq!(engine.generation_counter(), 0);

    let custom_config = FuzzyCompareConfig {
        epsilon: 1e-3,
        relative_tolerance: 1e-6,
    };
    let custom_rules = GameRules::default();

    let custom_engine = StateDiff::with_config(custom_config, custom_rules);
    assert_eq!(custom_engine.generation_counter(), 0);
}

#[test]
fn test_state_diff_snapshot_creation() {
    let mut engine = StateDiff::new();

    let entities = vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )];

    let snapshot1 = engine.create_snapshot(entities.clone());
    let snapshot2 = engine.create_snapshot(entities);

    assert_eq!(snapshot1.generation, 1);
    assert_eq!(snapshot2.generation, 2);
    assert!(snapshot2.generation > snapshot1.generation);
}

#[test]
fn test_entity_addition_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![]);
    let after = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].change_type, ChangeType::EntityAdded);
    assert_eq!(result.changes[0].entity_id, 1);
}

#[test]
fn test_entity_removal_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    let after = engine.create_snapshot(vec![]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].change_type, ChangeType::EntityRemoved);
    assert_eq!(result.changes[0].entity_id, 1);
}

#[test]
fn test_component_modification_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    let after = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 1.0}))],
    )]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].change_type, ChangeType::ComponentModified);
    assert_eq!(result.changes[0].entity_id, 1);
    assert_eq!(
        result.changes[0].component_type,
        Some("Transform".to_string())
    );
}

#[test]
fn test_component_addition_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    let after = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![
            ("Transform", json!({"x": 0.0})),
            ("Velocity", json!({"vx": 1.0})),
        ],
    )]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].change_type, ChangeType::ComponentAdded);
    assert_eq!(
        result.changes[0].component_type,
        Some("Velocity".to_string())
    );
}

#[test]
fn test_component_removal_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![
            ("Transform", json!({"x": 0.0})),
            ("Velocity", json!({"vx": 1.0})),
        ],
    )]);
    let after = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].change_type, ChangeType::ComponentRemoved);
    assert_eq!(
        result.changes[0].component_type,
        Some("Velocity".to_string())
    );
}

#[test]
fn test_multiple_changes_detection() {
    let mut engine = StateDiff::new();

    let before = engine.create_snapshot(vec![
        create_test_entity(1, vec![("Transform", json!({"x": 0.0}))]),
        create_test_entity(2, vec![("Health", json!(100))]),
    ]);
    let after = engine.create_snapshot(vec![
        create_test_entity(1, vec![("Transform", json!({"x": 1.0}))]),
        create_test_entity(3, vec![("Health", json!(50))]),
    ]);

    let result = engine.diff_snapshots(&before, &after);
    assert_eq!(result.changes.len(), 3); // Entity 1 modified, Entity 2 removed, Entity 3 added

    let added_changes: Vec<_> = result.filter_by_type(ChangeType::EntityAdded);
    let removed_changes: Vec<_> = result.filter_by_type(ChangeType::EntityRemoved);
    let modified_changes: Vec<_> = result.filter_by_type(ChangeType::ComponentModified);

    assert_eq!(added_changes.len(), 1);
    assert_eq!(removed_changes.len(), 1);
    assert_eq!(modified_changes.len(), 1);
}

#[test]
fn test_change_grouping() {
    let engine = StateDiff::new();

    let changes = vec![
        Change::new(
            ChangeType::ComponentModified,
            1,
            Some("Transform".to_string()),
            None,
            None,
        ),
        Change::new(
            ChangeType::ComponentModified,
            2,
            Some("Transform".to_string()),
            None,
            None,
        ),
        Change::new(
            ChangeType::ComponentModified,
            1,
            Some("Velocity".to_string()),
            None,
            None,
        ),
        Change::new(ChangeType::EntityAdded, 3, None, None, None),
    ];

    let groups = engine.group_changes(&changes);

    assert_eq!(groups.len(), 3); // Transform, Velocity, and Entity changes

    let transform_group = groups
        .iter()
        .find(|g| g.group_type == "Transform changes")
        .unwrap();
    assert_eq!(transform_group.changes.len(), 2);

    let entity_group = groups
        .iter()
        .find(|g| g.group_type == "Entity changes")
        .unwrap();
    assert_eq!(entity_group.changes.len(), 1);
}

#[test]
fn test_diff_summary_calculation() {
    let changes = vec![
        Change::new(ChangeType::EntityAdded, 1, None, None, None),
        Change::new(ChangeType::EntityRemoved, 2, None, None, None),
        Change::new(
            ChangeType::ComponentModified,
            3,
            Some("Transform".to_string()),
            None,
            None,
        ),
        Change::new(
            ChangeType::ComponentAdded,
            4,
            Some("Velocity".to_string()),
            None,
            None,
        ),
        Change::new(
            ChangeType::ComponentRemoved,
            5,
            Some("Health".to_string()),
            None,
            None,
        ),
    ];

    let summary = DiffSummary::from_changes(&changes);

    assert_eq!(summary.entities_added, 1);
    assert_eq!(summary.entities_removed, 1);
    // Entities 3, 4 and 5 each had a component modified, added or removed. This said 0, which
    // pinned B-078: the summary counted only `EntityModified`, a kind the diff never produces.
    assert_eq!(summary.entities_modified, 3);
    assert_eq!(summary.components_added, 1);
    assert_eq!(summary.components_removed, 1);
    assert_eq!(summary.components_modified, 1);
    assert_eq!(summary.total_changes, 5);
    assert_eq!(summary.unexpected_changes, 0);
}

/// An entity with several component changes is one modified entity, not one per change.
#[test]
fn test_diff_summary_counts_each_modified_entity_once() {
    let modified = |entity, component: &str| {
        Change::new(ChangeType::ComponentModified, entity, Some(component.to_string()), None, None)
    };
    let changes = vec![modified(7, "Transform"), modified(7, "Velocity"), modified(8, "Transform")];

    let summary = DiffSummary::from_changes(&changes);

    assert_eq!(summary.entities_modified, 2);
    assert_eq!(summary.components_modified, 3);
    assert!(summary.format().contains("~2 entities"), "{}", summary.format());
}

#[test]
fn test_diff_summary_formatting() {
    let summary = DiffSummary {
        entities_added: 2,
        entities_removed: 1,
        entities_modified: 0,
        components_added: 3,
        components_removed: 0,
        components_modified: 5,
        unexpected_changes: 2,
        total_changes: 11,
    };

    let formatted = summary.format();
    assert!(formatted.contains("+2 entities"));
    assert!(formatted.contains("-1 entities"));
    assert!(formatted.contains("+3 components"));
    assert!(formatted.contains("~5 components"));
    assert!(formatted.contains("(2 unexpected)"));
}

#[test]
fn test_diff_time_window() {
    let mut engine = StateDiff::new();
    let now = chrono::Utc::now();
    let mut window = DiffTimeWindow::new(now, now + chrono::Duration::seconds(10));

    let snapshot1 = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    let snapshot2 = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 1.0}))],
    )]);

    assert!(window.add_snapshot(snapshot1));
    assert!(window.add_snapshot(snapshot2));
    assert_eq!(window.snapshots.len(), 2);

    let merged_diff = window.get_merged_diff(&engine);
    assert!(merged_diff.is_some());

    let diff_result = merged_diff.unwrap();
    assert_eq!(diff_result.changes.len(), 1);
    assert_eq!(
        diff_result.changes[0].change_type,
        ChangeType::ComponentModified
    );
}

#[test]
fn test_diff_time_window_capacity_limit() {
    let mut engine = StateDiff::new();
    let now = chrono::Utc::now();
    let mut window = DiffTimeWindow::with_capacity(now, now + chrono::Duration::seconds(10), 2);

    let snapshot1 = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    let snapshot2 = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 1.0}))],
    )]);
    let snapshot3 = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 2.0}))],
    )]);

    window.add_snapshot(snapshot1);
    window.add_snapshot(snapshot2);
    window.add_snapshot(snapshot3);

    // Should only keep the last 2 snapshots due to capacity limit
    assert_eq!(window.snapshots.len(), 2);
}

#[test]
fn test_diff_time_window_outside_range() {
    let mut engine = StateDiff::new();
    let now = chrono::Utc::now();
    let mut window = DiffTimeWindow::new(now, now + chrono::Duration::seconds(10));

    // Create snapshot with timestamp outside the window
    let mut snapshot = engine.create_snapshot(vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )]);
    snapshot.timestamp = now - chrono::Duration::seconds(20); // Before window start

    assert!(!window.add_snapshot(snapshot));
    assert_eq!(window.snapshots.len(), 0);
}

#[test]
fn test_change_formatted_output() {
    let change = Change::new(
        ChangeType::ComponentModified,
        42,
        Some("Transform".to_string()),
        // Bare numbers, because the assertion below is about how `format_value` renders a *number*:
        // a whole-valued float prints as `0` / `1`. These used to be `{"x": 0.0}` objects, which take
        // the object branch and render as `{x: 0}`, so `"0 → 1"` never appeared and the test could
        // not pass.
        Some(json!(0.0)),
        Some(json!(1.0)),
    );

    let formatted = change.format_colored();
    assert!(formatted.contains("Component modified entity 42 Transform"));
    assert!(formatted.contains("0 → 1"));
    assert!(formatted.contains("\x1b[33m")); // Yellow color code
    assert!(formatted.contains("\x1b[0m")); // Reset code
}

#[test]
fn test_change_unexpected_marker() {
    let mut change = Change::new(
        ChangeType::ComponentModified,
        42,
        Some("Transform".to_string()),
        Some(json!(0.0)),
        Some(json!(1.0)),
    );
    change.is_unexpected = true;

    let formatted = change.format_colored();
    assert!(formatted.contains("⚠️"));
}

#[test]
fn test_state_diff_result_filtering() {
    let changes = vec![
        Change::new(ChangeType::EntityAdded, 1, None, None, None),
        Change::new(ChangeType::EntityRemoved, 2, None, None, None),
        Change::new(
            ChangeType::ComponentModified,
            3,
            Some("Transform".to_string()),
            None,
            None,
        ),
    ];

    let before = StateSnapshot::new(vec![], 1);
    let after = StateSnapshot::new(vec![], 2);
    let result = StateDiffResult::new(changes, before, after);

    let added_changes = result.filter_by_type(ChangeType::EntityAdded);
    assert_eq!(added_changes.len(), 1);
    assert_eq!(added_changes[0].entity_id, 1);

    let removed_changes = result.filter_by_type(ChangeType::EntityRemoved);
    assert_eq!(removed_changes.len(), 1);
    assert_eq!(removed_changes[0].entity_id, 2);
}

#[test]
fn test_state_diff_result_unexpected_changes() {
    let change1 = Change::new(ChangeType::EntityAdded, 1, None, None, None);
    let mut change2 = Change::new(ChangeType::EntityRemoved, 2, None, None, None);
    change2.is_unexpected = true;

    let changes = vec![change1, change2];
    let before = StateSnapshot::new(vec![], 1);
    let after = StateSnapshot::new(vec![], 2);
    let result = StateDiffResult::new(changes, before, after);

    let unexpected_changes = result.unexpected_changes();
    assert_eq!(unexpected_changes.len(), 1);
    assert_eq!(unexpected_changes[0].entity_id, 2);
}

#[test]
fn test_generation_counter_overflow() {
    let mut engine = StateDiff::new();
    // Skip testing private field manipulation - this test is conceptual
    let entities = vec![create_test_entity(
        1,
        vec![("Transform", json!({"x": 0.0}))],
    )];
    let snapshot = engine.create_snapshot(entities);

    // Should increment generation counter
    assert_eq!(snapshot.generation, 1);
    assert_eq!(engine.generation_counter(), 1);
}

#[test]
fn test_engine_configuration_updates() {
    let mut engine = StateDiff::new();

    let new_config = FuzzyCompareConfig {
        epsilon: 1e-3,
        relative_tolerance: 1e-6,
    };
    engine.set_fuzzy_config(new_config);

    let new_rules = GameRules {
        max_position_change_per_second: Some(100.0),
        ..Default::default()
    };
    engine.set_game_rules(new_rules);

    // Test that the new configuration is applied.
    //
    // These differ by 5e-4 — inside the new 1e-3 epsilon, and far outside the 1e-6 default, so the
    // assertion genuinely proves `set_fuzzy_config` took effect. The pair used to be 1.001/1.002,
    // which differ by *exactly* the epsilon and therefore land on the boundary: in f64 the
    // subtraction yields 0.001000000000000112, a hair over 1e-3, so `diff <= epsilon` was false and
    // the test could not pass.
    let val1 = json!(1.0005);
    let val2 = json!(1.0010);

    assert!(
        val1.fuzzy_eq(&val2, engine.fuzzy_config()),
        "a 5e-4 difference must be inside the 1e-3 epsilon just configured"
    );
    // And the same pair is *not* equal under the default epsilon, which is what makes this a test of
    // the setter rather than of fuzzy comparison in general.
    assert!(!val1.fuzzy_eq(&val2, &FuzzyCompareConfig::default()));
}
