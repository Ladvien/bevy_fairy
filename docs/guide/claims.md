# Changing the game, testing a claim, scoring anomalies

These three tools all keep state about the game between calls, and all three stop trusting that
state when the game process changes.

## `experiment`: change the game and undo it

`execute` carries the `actions`: a spawn, a modify, a delete or a batch. Component types are full
paths (`game::Crate`), never short names, because BRP takes full type paths and no game is guaranteed
to have a `Health`.

The reply lists what each action did, taken from the game's own answer. An action the game or the
server refuses is a failed result and the rest still run. A batch lists its own actions, and a batch
cannot contain a batch. An atomic batch stops at its first failure and reverses what it ran, marking
those results `rolled_back`.

A modify's result carries `read_back`: the written components as the game holds them a frame or so
later, and whether they `matches`. A value the game's own systems write again reads `matches: false`,
and the message says the write was accepted but is not held.

`undo` and `redo` step through the history and answer with an `action_type` of `undo` or `redo` and
`of_action`, the kind they stepped over. A redo of a spawn makes a different entity, and the redo
entries behind it follow the new one.

`history` and `clear_history` read and empty the list.

## `hypothesis`: test a claim against the running game

`test` takes a `description` and a `success_condition`. The condition is a string
(`entity_exists:game::Crate`, `component_equals:game::Score=100`) or an object whose `type` is one of
`entity_exists`, `component_equals`, `performance_within` (entity count or component count),
`action_succeeds`, `all`, `any` and `not`.

A condition that cannot be read as written, or that names a component the game never registered, is
refused before anything runs: a claim about a misspelled component would otherwise be REJECTED
whatever the game did.

Each iteration runs the `actions`, reads the condition, and then reverses what the actions spawned or
modified, so every iteration starts from the game as it was found. `game_state` counts what was
`actions_reversed` and what is `left_applied`: a delete cannot be put back, because a respawned entity
is a different one.

The reply's `statistics` counts each iteration as `held`, `violated` or `not_evaluated` — a refused
action is not a violation, it is an iteration that never read the condition — and the verdict is
`CONFIRMED`, `REJECTED` or `NOT_EVALUATED` at the `threshold` you set. The success rate and its 95%
Wilson interval are over the iterations that read the condition; the verdict is over every iteration,
so 90 held and 10 not evaluated is a success rate of 1.0 and is not CONFIRMED at 0.95.

`quick_test` fixes the budget at 10 iterations and 10 seconds. `stress_test` fuzzes the actions 1000
times unless `iterations` or `variation_type` say otherwise. `validate` checks that the claim parses
and runs nothing.

`variation_type` changes the numbers, strings and booleans inside `actions`, at any depth of a
component's value, and never which entity an action targets: `random` draws within
`numeric_min`..`numeric_max`, `string_pool` and `bool_probability`; `grid` walks `numeric_values`,
`string_values` and `bool_values`, for the kinds of value the actions write that it was given values
for; `boundary` tries the edge values the `include_*` flags name; `fuzz` mutates the actions up to
`max_mutations` times, each with `mutation_probability`. `seed` makes a run repeatable.

## `detect_anomaly`: score what looks wrong

`action` picks the operation: `detect` reads every entity once, `configure` sets the thresholds,
`status` lists what each detector reads.

The three detectors read components by the last segment of their type path: the linear vector of a
component named `Velocity` (scored against that entity's own recent speeds, including the one just
read), the current field of `Health` beside a boolean `Alive`, and the number of entities across
`detect` calls. Speeds and the entity count are judged only after `min_samples` readings, so the first
calls judge nothing.

Severity is how far past its threshold a reading is: 0.5 at twice the threshold. A state
inconsistency carries a fixed 0.9, because it is a rule's rating and not a measurement.

A `z_score_threshold` above what the window can reach is never exceeded, and `detect` then reports
that detector as not evaluated rather than reporting nothing found: among n samples the largest
possible score is `(n - 1) / sqrt(n)`, which is 2.85 among 10 and 9.9 among 100.

## A game restart is a different game

`bevy_fairy/status` reports an `instance`: one value per game process, unchanged for the game's life
and different after a restart. Each of these three tools keeps state keyed on entity ids, and ids from
one process mean nothing in another, so a restart discards that state:

- `experiment`'s `undo` and `redo` are refused after a restart, because the history names entities
  that no longer exist.
- `observe` with `diff: true` treats the first read after a restart as a new baseline, saying so.
- `detect_anomaly` drops its samples, so the new game is not scored against the old one's speeds.

A game whose plugin is older than 0.11 reports no `instance`, so the server cannot tell a restart;
those three tools run as before and say `game_instance_unverified` in the reply.
