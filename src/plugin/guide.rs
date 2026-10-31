//! **The step you are on, shown where the work is** — the agent-to-human half of the channel.
//!
//! An agent driving a Bevy app can already post input and read frames back. What it cannot do is
//! *tell the person at the keyboard what to try*, so every instruction goes to a terminal they have
//! to look away from their work to read, and every answer comes back as prose the agent then has to
//! guess its way from. This posts a script the app renders one step at a time, watches a stated
//! condition, and records what actually happened.
//!
//! # A step is a guided-exploration card, and that shape is forty years old
//!
//! Carroll's minimalist instruction (`10.14434/ijdl.v5i2.12887`) put adults in front of unfamiliar
//! software with 25 cards in place of ~100 manual pages, and measured them making *"more progress in
//! less time"*, recognising errors better and recovering *"more often and more rapidly"*. A card
//! carries brief hints, **a checkpoint**, and **error recognition and recovery information** — and
//! that third field is the one Chauvergne et al. 2023 (`10.1145/3544548.3581211`) could not find in a
//! single one of twenty-one shipped tutorials they reviewed: *"we did not find any tutorial that
//! gives detailed instructions"* on correcting a wrong action.
//!
//! So [`Step`] is that card, field for field, and the checkpoint is what this module watches.
//!
//! # One step, pushed, never a script to read first
//!
//! Andersen et al. 2012 (`10.1145/2207676.2207687`, N = 45,318) crossed four tutorial variables
//! across three games. In the complex, unconventional interface — the condition an editor is in —
//! giving instructions *"as closely as possible to when they were needed, rather than out of context
//! in an up-front manual, increased play time by 16% and progress by 40%."*
//!
//! Two of their negative results shape this as much as that one:
//!
//! - **Restricting freedom to make the user perform the step had no effect in any of the three
//!   games.** So this never gates input, and the app stays fully usable with a step on screen.
//! - **An on-demand help button cost 12% of levels completed and 15% of play time**, and only 31% of
//!   players ever clicked it. So there is no help affordance here to go and ask: the current step is
//!   pushed, and a caller that wants it gone hides it.
//!
//! # What comes back is `k/n`, never a boolean
//!
//! Bryant, *Game Testing All in One* 4e, on reproduction rate: a tester who ran the steps twice and
//! saw the bug twice could *"reasonably"* report 100%, and it is *"just as likely that the bug is
//! only reproducible 50% of the time or less, and you just got lucky, as though you had flipped a
//! penny and got it to land heads-up twice in a row. For this reason, many QA labs report the
//! reproduction rate as the number of observed occurrences over the number of attempts."*
//!
//! [`StepRecord`] counts runs and passes across every run of a script, which is the single thing a
//! machine watching the checkpoint buys that asking the person does not.

use std::collections::{HashMap, VecDeque};

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy::remote::{error_codes, BrpError, BrpResult};
use serde::Deserialize;
use serde_json::{json, Value};

/// How long the confirmation of a finished step stays up before the next one appears.
///
/// van der Meij & van der Meij 2016 (`10.1007/s11251-016-9394-9`) build software-training video from
/// demonstration-based training and put **two-second pauses** between sections; Moreno's two
/// experiments found animations with 2-5 s pauses were judged *"less difficult and requiring less
/// mental effort"* and produced significantly higher transfer.
///
/// It is also what makes the overlay a **delta rather than a steady state**: `docs/ui.md` §3.4 in the
/// host repo records Lewandowska's finding that a constant peripheral element habituates away and
/// stops working at the moment it matters. Arriving-and-leaving is the part that gets read.
pub const BEAT_SECONDS: f32 = 2.0;

/// One step of a script — Carroll's card, field for field.
///
/// **Keep the text ASCII.** Bevy's embedded default font is 95 codepoints; an em-dash or a curly
/// quote draws as tofu in any host that has not installed a font of its own, and this crate cannot
/// ship one without widening the dependency list `tests/leaf.rs` pins.
///
/// # `deny_unknown_fields`, because every field here is `#[serde(default)]`
///
/// A misspelled key is otherwise **silent**: `crates/bevy_grimoire_mapper/guides/label_a_mesh.json` shipped
/// two steps whose checkpoint arguments were spelled `args` instead of `with`, so `with` came out
/// `None`, the condition was handed `null`, and both steps could never pass — a guide that looked
/// complete and could not be walked. There is nothing to weigh here: an unknown key in a script is
/// always a mistake, and refusing it at the door is the only place the author is still watching.
///
/// [`GuideParams`] stays permissive on purpose — script files carry top-level keys that are not
/// `Step` fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The sub-goal, in a few words. van der Meij: labelling each step of a procedure
    /// *"effectively creates a series of sub-goals"* and significantly improved knowledge
    /// development.
    pub label: String,
    /// One clause saying **why** this is being asked.
    ///
    /// Not decoration. Choong et al. 2025 (`10.1145/3706598.3713576`) interviewed twenty people about
    /// an AI making suggestions inside a game: 13 of 20 needed to know why before the suggestion
    /// helped them, and *"If the AI does not give the reason why [it's] making this suggestion, it
    /// will not help the player."*
    #[serde(default)]
    pub goal: String,
    /// Two to four imperative hints. **Three items is the ceiling worth designing to** — working
    /// memory holds 2-4 actively processed elements for about 30 seconds (Ouellette et al. 2019,
    /// `10.1145/3337722.3337725`), which is also why the step stays on screen while the person acts
    /// rather than being read and dismissed.
    #[serde(default, rename = "do")]
    pub hints: Vec<String>,
    /// The name of a host-registered condition — see [`Checkpoints`].
    ///
    /// `None` is a real state: a step whose result only a person can judge ("does this look right?")
    /// has no machine check and waits for an explicit advance.
    #[serde(default)]
    pub checkpoint: Option<String>,
    /// Arguments for the checkpoint, handed to it as `In<Value>`.
    ///
    /// **This is what lets a condition be as strong as the step that claims it.** Without it every
    /// checkpoint has to be a fixed sentence, so a host registers vague ones -- and a vague condition
    /// is satisfied by things the step never asked for. That is not hypothetical: `the tile is saved`
    /// was true whenever *any* already-saved tile was open, so a step reported a pass for a tile that
    /// was never made, and the transcript recorded 1/1 for work that did not happen. The transcript
    /// being trustworthy is the entire value of this module, so that is the worst thing it can do.
    ///
    /// Prefer arguments that make a condition **monotonic** -- "the kit has at least N tiles" cannot
    /// be re-satisfied by revisiting old work, and "the open tile is saved" can.
    #[serde(default)]
    pub with: Option<Value>,
    /// What to do when the checkpoint does not happen. The field twenty-one shipped tutorials did
    /// not have.
    #[serde(default)]
    pub recovery: String,
}

/// What happened to one step, across every run of the script.
#[derive(Debug, Clone, Default)]
pub struct StepRecord {
    pub label: String,
    /// How many times this step has been the current one.
    pub runs: u32,
    /// How many of those times its checkpoint passed. The `k` of `k/n`.
    pub passes: u32,
    /// Seconds from becoming current to passing, last time it did. A step nobody can complete shows
    /// up as a stall rather than as a silence.
    pub seconds: f32,
}

/// The script, where the person is in it, and what happened.
///
/// Registered by [`crate::DebuggerPlugin`], and **independently `init_resource`-able**: a host's
/// headless harness may want the resource without the plugin, because the plugin's transport binds a
/// port and a test process builds several `App`s.
#[derive(Resource, Default)]
pub struct Guide {
    steps: Vec<Step>,
    /// Which step is current. Equal to `steps.len()` when the script is finished.
    at: usize,
    transcript: Vec<StepRecord>,
    /// Seconds the current step has been up.
    elapsed: f32,
    /// Counts down after a step passes, holding its confirmation on screen. See [`BEAT_SECONDS`].
    beat: f32,
    /// The step being confirmed during the beat, and **whether it actually passed**.
    ///
    /// The bool is not decoration. Without it the card said `OK <step>` in green for a step the
    /// person had *skipped* — and a skip is how they say "this made no sense", which is the one
    /// signal Choong et al. found people withhold (18 of 20 followed an ambiguous suggestion rather
    /// than report it). Answering that with a green tick tells them they did fine, which is the
    /// opposite of the thing the escape hatch is for. Found in a devshot capture; no test had an
    /// opinion, because both branches rendered.
    finished: Option<(String, bool)>,
    /// Which step index has already had a **non-advancing** answer sent on the watch stream.
    ///
    /// [`watch_guide`] re-runs every frame, so any answer that does not move `at` would be re-sent
    /// sixty times a second — a step with no checkpoint, or the end of the script. Both are announced
    /// once per index and then park.
    ///
    /// **This makes the watch stream single-consumer**, and deliberately: there is no per-request
    /// identity to key on (`process_single_ongoing_watching_request` passes the handler the params
    /// and nothing else), so a second watcher attached after an announcement never sees it. The
    /// stream is a notification channel; `bevy_fairy/guide` with `{"read": true}` is the state, and
    /// anything that needs to be sure asks for that.
    announced: Option<usize>,
    /// Passes the app recorded that the watch stream has not told anyone about yet, oldest first.
    ///
    /// **The checkpoint is evaluated by [`evaluate_checkpoint`], once a frame, whether or not anyone
    /// is watching.** It used to be evaluated only by the watch stream, and the MCP `guide` tool can
    /// open no stream, so through it a step with a checkpoint never advanced while its description
    /// said the app moved on by itself (B-127). The stream is now a pure reporter: it drains this.
    unreported: VecDeque<String>,
    /// Why the current step's checkpoint could not be run on the last frame that tried: a name no
    /// host registered (only a script posted from Rust can say one; the BRP method refuses it) or a
    /// checkpoint system that failed. `report` says it, so a `read` is not silent about a step that
    /// will never move.
    checkpoint_error: Option<String>,
    /// How many scripts have been posted. The overlay's render key includes it, because a script
    /// re-posted while step 1 is up has the same `at` and the same phase as the one it replaced, so
    /// the card kept the old step's text.
    generation: u64,
    /// Whether the overlay draws. Public so a host may bind a key to it — this crate claims none,
    /// because the host's own key census owns that space.
    pub visible: bool,
}

impl Guide {
    /// Replace the script. Counts survive a re-post of the **same** labels, which is what makes
    /// `k/n` mean anything across runs.
    pub fn post(&mut self, steps: Vec<Step>) {
        let same = self.transcript.len() == steps.len()
            && self
                .transcript
                .iter()
                .zip(steps.iter())
                .all(|(r, s)| r.label == s.label);
        if !same {
            self.transcript = steps
                .iter()
                .map(|s| StepRecord {
                    label: s.label.clone(),
                    ..Default::default()
                })
                .collect();
        }
        self.steps = steps;
        self.at = 0;
        self.elapsed = 0.0;
        self.beat = 0.0;
        self.finished = None;
        self.announced = None;
        self.unreported.clear();
        self.checkpoint_error = None;
        self.generation += 1;
        self.visible = true;
        self.enter();
    }

    /// Take the script down. The transcript survives, because it is the answer.
    pub fn clear(&mut self) {
        self.steps.clear();
        self.at = 0;
        self.visible = false;
        self.finished = None;
        self.beat = 0.0;
        self.unreported.clear();
        self.checkpoint_error = None;
    }

    /// The step being worked on, if any.
    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.at)
    }

    /// How far through, as `(step, of)`, one-based for a reader.
    pub fn position(&self) -> (usize, usize) {
        (self.at.min(self.steps.len()) + 1, self.steps.len())
    }

    pub fn transcript(&self) -> &[StepRecord] {
        &self.transcript
    }

    /// Count this step as attempted. Called on arrival, not on completion, so a step that is never
    /// finished still has an `n`.
    fn enter(&mut self) {
        if let Some(r) = self.transcript.get_mut(self.at) {
            r.runs += 1;
        }
        self.elapsed = 0.0;
    }

    /// Move past the current step, recording whether its checkpoint was met.
    ///
    /// `passed` is false for an explicit skip, which is the escape hatch Choong et al. found is
    /// needed: 18 of 20 of their participants followed an AI suggestion rather than report that it
    /// made no sense, so a script with no way to say "this step is wrong" collects agreement instead
    /// of data.
    pub fn advance(&mut self, passed: bool) -> Option<String> {
        let label = self.steps.get(self.at).map(|s| s.label.clone())?;
        if let Some(r) = self.transcript.get_mut(self.at) {
            if passed {
                r.passes += 1;
                r.seconds = self.elapsed;
            }
        }
        self.at += 1;
        self.checkpoint_error = None;
        self.finished = Some((label.clone(), passed));
        self.beat = BEAT_SECONDS;
        if self.at < self.steps.len() {
            self.enter();
        }
        Some(label)
    }

    /// The transcript as JSON, `k/n` per step.
    pub fn report(&self) -> Value {
        let rows: Vec<Value> = self
            .transcript
            .iter()
            .map(|r| {
                json!({
                    "step": r.label,
                    "passes": r.passes,
                    "runs": r.runs,
                    "seconds": r.seconds,
                })
            })
            .collect();
        let (step, of) = self.position();
        // **What the current step needs, so a `read` is a complete answer.**
        //
        // Without this the watch stream was the only place `waiting_on_a_person` was ever said, and
        // it is announced once per step — so a client that reconnected after the announcement (a
        // dropped connection, a crashed watcher, a second tool attaching later) could not tell a step
        // that needs a human call from one whose condition simply has not arrived. It waited on a
        // machine that was waiting on it.
        //
        // The fix is not to re-announce on the stream: there is no per-request identity to key that
        // on, and re-announcing per frame is the flood `announced` exists to stop. It is to make the
        // state query answer the question. The stream stays a notification channel; this is the state.
        let waiting = self
            .current()
            .map(|s| s.checkpoint.is_none())
            .unwrap_or(false);
        let mut report = json!({
            "at": step,
            "of": of,
            "step": self.current().map(|s| s.label.clone()),
            "waiting_on_a_person": waiting,
            // The condition the app is evaluating every frame for this step; null when only a person
            // can judge it.
            "checkpoint": self.current().and_then(|s| s.checkpoint.clone()),
            "steps": rows,
        });
        if let Some(why) = &self.checkpoint_error {
            report["checkpoint_error"] = json!(why);
        }
        report
    }
}

/// The conditions a host is willing to have watched, by name.
///
/// **The host owns the vocabulary, and that is the whole seam.** This crate cannot know what a tile
/// or a selection is, and must not learn — it stays an engine-level plugin with five dependencies and
/// no reach into any particular app. So a host registers one-shot systems that answer `bool`, and a
/// step names one. The plugin runs it with the exclusive `World` a BRP handler already has.
///
/// It is the same shape [`crate::PendingInput`]'s public `queue_*` methods are: the plugin owns the
/// machinery, the host owns its own words.
///
/// ```rust,no_run
/// # use bevy::prelude::*;
/// # use bevy_fairy::Checkpoints;
/// # use serde_json::Value;
/// # #[derive(Resource, Default)] struct Tile { members: usize }
/// # let mut app = App::new();
/// # app.init_resource::<Tile>().init_resource::<Checkpoints>();
/// // Every checkpoint takes `In<Value>`, whether or not it reads it: one shape, so a step can
/// // always say what it means.
/// let id = app.register_system(|args: In<Value>, tile: Res<Tile>| {
///     let want = args.0.get("n").and_then(|n| n.as_u64()).unwrap_or(2) as usize;
///     tile.members >= want
/// });
/// app.world_mut()
///     .resource_mut::<Checkpoints>()
///     .register("tile has two members", id);
/// ```
#[derive(Resource, Default)]
pub struct Checkpoints(HashMap<String, SystemId<In<Value>, bool>>);

impl Checkpoints {
    pub fn register(&mut self, name: impl Into<String>, system: SystemId<In<Value>, bool>) {
        self.0.insert(name.into(), system);
    }

    pub fn get(&self, name: &str) -> Option<SystemId<In<Value>, bool>> {
        self.0.get(name).copied()
    }

    /// Every registered name, sorted — so a refusal can say what *would* have worked instead of only
    /// what did not.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.0.keys().cloned().collect();
        out.sort();
        out
    }
}

fn invalid_params(message: String) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message, data: None }
}

/// What `bevy_fairy/guide` accepts.
#[derive(Debug, Default, Deserialize)]
pub struct GuideParams {
    /// Post or replace the script.
    #[serde(default)]
    pub steps: Option<Vec<Step>>,
    /// Return the transcript without changing anything.
    #[serde(default)]
    pub read: bool,
    /// Take the script down.
    #[serde(default)]
    pub clear: bool,
    /// Move past the current step without its checkpoint passing — the "this step made no sense"
    /// escape hatch. Recorded as an attempt that did not pass.
    #[serde(default)]
    pub skip: bool,
    /// Show or hide the overlay without discarding the script.
    #[serde(default)]
    pub visible: Option<bool>,
}

/// BRP handler: `bevy_fairy/guide`.
///
/// Posts, reads, skips, clears. The reply says what was **accepted**, never what has happened — the
/// convention `handle_input` and `handle_screenshot` already hold to, and for the same reason: a
/// method that claims the person has done something is the failure this crate exists to prevent.
///
/// One verb per call (`steps`, `read`, `clear` or `skip`); a call naming two is refused, because the
/// handler used to run the first it met and answer `success: true` for all of them. `visible` goes
/// with `steps` or `skip` and is applied after them, so `{steps, visible: false}` posts a hidden
/// script; with `read` or `clear` it has nothing to act on and is refused.
pub fn handle_guide(
    In(params): In<Option<Value>>,
    mut guide: ResMut<Guide>,
    checkpoints: Res<Checkpoints>,
) -> BrpResult {
    // `serde_json::Error` has no `From` into `BrpError`, so `?` cannot carry it.
    let p: GuideParams = match params.as_ref() {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| invalid_params(format!("invalid guide params: {e}")))?,
        None => GuideParams::default(),
    };

    let verbs: Vec<&str> = [
        p.steps.is_some().then_some("steps"),
        p.read.then_some("read"),
        p.clear.then_some("clear"),
        p.skip.then_some("skip"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if verbs.len() > 1 {
        return Err(invalid_params(format!(
            "one verb per call, got {}: only the first would have run and the reply would have \
             said success for all of them. Send them as separate calls. Example: {{\"skip\": true}}",
            verbs.join(" and ")
        )));
    }
    if p.visible.is_some() && (p.read || p.clear) {
        return Err(invalid_params(format!(
            "`visible` does nothing with `{}`: read changes nothing and clear already takes the \
             card down. Use it alone, or with `steps` or `skip`. Example: {{\"visible\": false}}",
            if p.read { "read" } else { "clear" }
        )));
    }

    if p.read {
        return Ok(json!({ "success": true, "guide": guide.report() }));
    }
    if p.clear {
        guide.clear();
        return Ok(json!({ "success": true, "message": "script taken down" }));
    }
    if p.skip {
        // Checked before anything is applied: a refused call changes nothing.
        if guide.current().is_none() {
            return Err(invalid_params(
                "no current step to skip: no script is posted, or it is finished. Read the guide \
                 to see where it is. Example: {\"read\": true}"
                    .to_string(),
            ));
        }
        if let Some(v) = p.visible {
            guide.visible = v;
        }
        let label = guide.advance(false);
        return Ok(json!({
            "success": true,
            "skipped": label,
            "guide": guide.report(),
        }));
    }
    if let Some(steps) = p.steps {
        if steps.is_empty() {
            return Err(invalid_params(
                "a script needs at least one step; use clear to take one down".to_string(),
            ));
        }
        // A name nobody registered would wait for ever: the app evaluates a step's checkpoint every
        // frame and has nothing to evaluate. Refused here, while the person is not yet standing in
        // front of the card.
        if let Some((label, name)) = steps.iter().find_map(|s| {
            let name = s.checkpoint.as_deref()?;
            checkpoints.get(name).is_none().then(|| (s.label.as_str(), name))
        }) {
            let known = checkpoints.names();
            return Err(invalid_params(format!(
                "step \"{label}\" names checkpoint `{name}`, which this app has not registered, so \
                 it would wait for ever. This app registers: {}. Name one of those, or use \
                 checkpoint: null for a step only the person can judge. Nothing was posted.",
                if known.is_empty() { "none".to_string() } else { known.join(", ") }
            )));
        }
        let n = steps.len();
        guide.post(steps);
        if let Some(v) = p.visible {
            guide.visible = v;
        }
        return Ok(json!({
            "success": true,
            "message": format!("{n} step(s) posted; the app shows the first one"),
        }));
    }
    if let Some(v) = p.visible {
        guide.visible = v;
    }
    Ok(json!({ "success": true, "guide": guide.report() }))
}

/// Runs the current step's checkpoint, once per frame while a step with one is current, and advances
/// the step when it passes. Registered by [`crate::DebuggerPlugin`] in `Last`, so it sees the frame's
/// finished state and runs ahead of the BRP handlers in `RemoteLast`.
///
/// **This is the one place a checkpoint is evaluated.** It is a plain system that queues the
/// exclusive work only when a step with a checkpoint is current, so an app with no script (the
/// resting state of every host that has the plugin on) pays for one `Res` read.
pub fn evaluate_checkpoint(guide: Res<Guide>, mut commands: Commands) {
    if guide.current().is_some_and(|step| step.checkpoint.is_some()) {
        commands.queue(|world: &mut World| {
            let outcome = run_current_checkpoint(world);
            if let Some(mut guide) = world.get_resource_mut::<Guide>() {
                guide.checkpoint_error = outcome.err().map(|e| e.message);
            }
        });
    }
}

/// The refusal for a checkpoint name nothing registered.
fn unregistered(world: &World, name: &str) -> BrpError {
    let known = world.get_resource::<Checkpoints>().map(Checkpoints::names).unwrap_or_default();
    invalid_params(format!(
        "no checkpoint named `{name}`. This host registers: {}. A script naming a condition \
         nobody watches would wait for ever.",
        if known.is_empty() { "none".to_string() } else { known.join(", ") }
    ))
}

/// Evaluates the current step's checkpoint once. A pass advances the step, records it, and queues it
/// for the watch stream; a false answer changes nothing; an error is returned for the caller to keep.
fn run_current_checkpoint(world: &mut World) -> Result<(), BrpError> {
    let Some((name, label, args)) = world.get_resource::<Guide>().and_then(|guide| {
        let step = guide.current()?;
        Some((step.checkpoint.clone()?, step.label.clone(), step.with.clone().unwrap_or(Value::Null)))
    }) else {
        return Ok(());
    };
    let Some(system) = world.get_resource::<Checkpoints>().and_then(|c| c.get(&name)) else {
        return Err(unregistered(world, &name));
    };
    match world.run_system_with(system, args) {
        Ok(true) => {
            let mut guide = world.resource_mut::<Guide>();
            guide.advance(true);
            guide.unreported.push_back(label);
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => Err(BrpError {
            code: error_codes::INTERNAL_ERROR,
            message: format!("checkpoint `{name}` could not run: {e}"),
            data: None,
        }),
    }
}

/// BRP handler: `bevy_fairy/guide+watch`.
///
/// **A watching method**, so the engine owns the waiting. `RemotePlugin` re-runs this every frame in
/// `RemoteLast` with exclusive `World`, parks the request while it answers `Ok(None)`, and reaps it
/// if the caller goes away (`bevy_remote-0.20.1/src/lib.rs:1559`). That is a notification channel
/// with lifecycle management already written, tested by the engine, and costing no new dependency —
/// which is what keeps `tests/leaf.rs` green.
///
/// **It reports; it does not evaluate.** [`evaluate_checkpoint`] advances a step on the frame its
/// checkpoint passes, watched or not, and this answers `passed` for each pass the app recorded that
/// no earlier turn of the stream told anyone about (a watcher that attaches late is told what it
/// missed, in order, with `guide.at` showing where the script is now). A step with no checkpoint
/// answers once, naming itself as needing a human call: there is nothing to watch, and pretending
/// otherwise would park the request forever.
pub fn watch_guide(In(_params): In<Option<Value>>, world: &mut World) -> BrpResult<Option<Value>> {
    let Some(mut guide) = world.get_resource_mut::<Guide>() else {
        return Err(BrpError {
            code: error_codes::INTERNAL_ERROR,
            message: "no Guide resource; add DebuggerPlugin or init_resource::<Guide>()".to_string(),
            data: None,
        });
    };

    if let Some(label) = guide.unreported.pop_front() {
        let report = guide.report();
        return Ok(Some(json!({ "passed": label, "guide": report })));
    }

    let (name, label) = match guide.current() {
        Some(step) => (step.checkpoint.clone(), step.label.clone()),
        None => {
            // Announce the end once. Answering it every frame would push sixty identical frames a
            // second down a stream whose whole contract is "something happened".
            if !announce_once(world) {
                return Ok(None);
            }
            let report = world.resource::<Guide>().report();
            return Ok(Some(json!({ "done": true, "guide": report })));
        }
    };

    // No checkpoint: only a person can judge this one. Say so once, then park — the step is advanced
    // by an explicit `skip`, which moves `at` and re-arms this.
    let Some(name) = name else {
        if !announce_once(world) {
            return Ok(None);
        }
        return Ok(Some(json!({
            "waiting_on_a_person": true,
            "step": label,
            "message": "this step has no checkpoint; advance it with skip once it has been judged",
        })));
    };

    // A checkpoint the app cannot run is said once per step, not once per frame.
    let problem = if world.get_resource::<Checkpoints>().is_some_and(|c| c.get(&name).is_some()) {
        world.resource::<Guide>().checkpoint_error.clone().map(|message| BrpError {
            code: error_codes::INTERNAL_ERROR,
            message,
            data: None,
        })
    } else {
        Some(unregistered(world, &name))
    };
    match problem {
        Some(error) if announce_once(world) => Err(error),
        // Not yet, or already said. The request stays open and this runs again next frame.
        _ => Ok(None),
    }
}

/// Has this step index already been announced on the watch stream? Marks it if not.
///
/// Returns `true` exactly once per index — see [`Guide::announced`].
fn announce_once(world: &mut World) -> bool {
    let mut guide = world.resource_mut::<Guide>();
    let at = guide.at;
    if guide.announced == Some(at) {
        return false;
    }
    guide.announced = Some(at);
    true
}

/// Ticks the beat between steps, so a finished step's confirmation is seen before the next arrives.
pub fn tick_guide(time: Res<Time>, mut guide: ResMut<Guide>) {
    let dt = time.delta_secs();
    // Guarded so a guide with no script writes nothing at all — the resting state of every host that
    // has this plugin on and is not being guided.
    if guide.beat > 0.0 {
        guide.beat = (guide.beat - dt).max(0.0);
        if guide.beat == 0.0 {
            guide.finished = None;
        }
    } else if guide.current().is_some() {
        guide.elapsed += dt;
    }
}

// ── the overlay ──────────────────────────────────────────────────────────────────────────────────

/// The overlay's root. `Display::None` when there is nothing to say.
#[derive(Component)]
pub struct GuideOverlay;

/// Where the step's lines are rebuilt, so the frame around them is spawned once.
#[derive(Component)]
struct GuideBody;

/// What the overlay was last built for, so it is rebuilt on a change and not per frame.
#[derive(Resource, Default, PartialEq, Eq)]
struct Showing(Option<(usize, bool, u64)>);

/// Where the card sits, in logical pixels from the top of the window.
///
/// **This exists because the plugin cannot know a host's chrome, and the first capture proved it.**
/// The default put the card at 12 px, which in `bevy_grimoire_mapper` is exactly the tab bar: the shot came
/// back with ANIM ghosting through STEP 1 OF 8 at 0.92 alpha. Guessing a bigger number would only
/// move the collision to a host with a taller bar.
///
/// So the host says. [`crate::DebuggerPlugin`] `init_resource`s this — insert-if-absent — so a host
/// that `insert_resource`s its own value wins whichever order the plugins are added in.
#[derive(Resource)]
pub struct GuidePlacement {
    /// Logical pixels from the top of the window to the top of the card.
    pub top: f32,
    /// Card width in logical pixels. Wide enough for a hint on one line, narrow enough that the eye
    /// does not have to travel: van der Meij's signalling argument is about structure, and a line
    /// long enough to lose your place in defeats it.
    pub width: f32,
}

impl Default for GuidePlacement {
    fn default() -> Self {
        GuidePlacement { top: 12.0, width: 440.0 }
    }
}

/// **No camera of its own, and that is deliberate.**
///
/// A `Camera2d` spawned here would be a second camera in somebody else's app, and Bevy 0.20's
/// `Single<..>` silently *skips its system* on a non-unique match — so a host with a
/// `Single<&Camera>` anywhere would stop working the moment this plugin was switched on, with no
/// error to read. The host repo has paid for that class twice and its answer is on the record: a
/// capture camera carries no shared marker and every query filters positively on the host's own.
///
/// So this spawns into whatever UI tree the host already renders. An app with no UI camera shows
/// nothing, which is honest and costs that app nothing.
fn spawn_guide_overlay(mut commands: Commands) {
    commands
        .spawn((
            GuideOverlay,
            Node {
                position_type: PositionType::Absolute,
                // Top centre: out of the way of the panels most editors put down the sides, and in
                // the half of the screen the work is usually not in. The exact offset is the host's
                // — see `GuidePlacement`, and the capture that made it a resource.
                left: Val::Percent(50.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                display: Display::None,
                ..default()
            },
            // **A container that does not eat clicks.** A UI node over the world swallows every
            // pick underneath it; the host's app has to stay fully usable with a step on screen,
            // because Andersen et al. measured no benefit at all from restricting freedom.
            bevy::picking::Pickable::IGNORE,
            // Above a typical editor's panels without being a modal: this is a card, not a scrim.
            GlobalZIndex(500),
            // Nearly opaque. The first capture was 0.92 over a tab bar and the labels read straight
            // through it — at which point the card is competing with the thing it sits on rather
            // than being read. It no longer overlaps anything, and it no longer needs to be trusted
            // not to.
            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.97)),
            // A hairline, so the card reads as one object rather than as text that has come loose.
            BorderColor::all(Color::srgba(0.92, 0.80, 0.35, 0.45)),
        ))
        .with_children(|p| {
            p.spawn((GuideBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), ..default() }));
        });
}

/// Paints the current step, or the confirmation of the one just finished.
///
/// **One step, never the script.** Llanos & Jorgensen 2011 found players accept a superimposed
/// overlay while it is "clearly motivated, and provides relevant and sufficient information" and
/// that "once the players receive more information than they need, superimposed UI elements become
/// annoying". Andersen's +40% is the same claim with a number on it.
fn paint_guide(
    mut commands: Commands,
    guide: Res<Guide>,
    place: Res<GuidePlacement>,
    mut showing: ResMut<Showing>,
    mut roots: Query<&mut Node, With<GuideOverlay>>,
    bodies: Query<Entity, With<GuideBody>>,
) {
    let want = guide.visible && (guide.current().is_some() || guide.finished.is_some());
    for mut node in &mut roots {
        let display = if want { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        // Applied here rather than at spawn, so a host may move the card while it is up — and so
        // there is one place the placement is read instead of a spawn-time copy that goes stale.
        // Written only on a change: `Node` is change-detected and Bevy relayouts on a touch.
        let top = Val::Px(place.top);
        let width = Val::Px(place.width);
        let centre = UiRect::left(Val::Px(-place.width / 2.0));
        if node.top != top || node.width != width || node.margin != centre {
            node.top = top;
            node.width = width;
            node.margin = centre;
        }
    }

    // Rebuilt on a change of step or of phase, never per frame.
    let key = want.then_some((guide.at, guide.finished.is_some(), guide.generation));
    if showing.0 == key {
        return;
    }
    showing.0 = key;
    let Some(_) = key else { return };

    let (step_no, of) = guide.position();
    let finished = guide.finished.clone();
    let current = guide.current().cloned();

    for body in &bodies {
        commands.entity(body).despawn_related::<Children>();
        let finished = finished.clone();
        let current = current.clone();
        commands.entity(body).with_children(|p| {
            // The beat: the step that just passed, held for a moment. Arriving-and-leaving is what
            // gets read; a banner that never changes stops being one.
            if let Some((done, passed)) = finished {
                let (mark, colour) = if passed {
                    ("OK", Color::srgb(0.52, 0.78, 0.48))
                } else {
                    // Muted, and the word says what happened. Not red: skipping is a legitimate
                    // answer -- it is what a step with no checkpoint is *for* -- and scolding
                    // somebody for using the escape hatch is how you stop them using it.
                    ("SKIPPED", Color::srgb(0.68, 0.66, 0.62))
                };
                line(p, format!("{mark}  {done}"), colour, 13.0);
                return;
            }
            let Some(step) = current else { return };
            line(p, format!("STEP {step_no} OF {of}   {}", step.label), Color::srgb(0.92, 0.80, 0.35), 13.0);
            // **Say when the step is waiting on the person, because it looks identical when it is
            // not.** A step with no checkpoint never advances on its own -- that is what makes it the
            // one a machine cannot judge -- and until this line existed the card for it was the same
            // card as one watching a condition. So the reasonable reading was "the editor has not
            // noticed yet", and the author sat in front of a step that was sitting in front of them.
            //
            // Found from the keyboard, mid-run, by somebody who had been told this would happen and
            // still could not tell it was happening. The fact was on the wire the whole time
            // (`waiting_on_a_person`); it was never anywhere a person looks.
            //
            // **It names the agent because this crate binds no key, and that is the host's call.**
            // `Guide::advance` is public precisely so a host can wire "I am happy" and "this made no
            // sense" to its own keys. Where one has not, saying "press something" would be a lie, and
            // an author who presses everything looking for it is worse off than one who is told
            // plainly that the answer goes back the way the question came.
            // **In a colour nothing else on the card uses, at the title's own size.** The first
            // version rendered this in the title gold two points smaller, and an author sitting in
            // front of exactly this step read the card as "the editor has not noticed yet" and
            // reported being stuck — the line was there and did not land (2026-08-14 guided run).
            // Camouflage is the failure mode of a warning that shares its palette.
            if step.checkpoint.is_none() {
                line(
                    p,
                    "-> yours to judge: no key advances this step. Answer the agent in chat."
                        .to_owned(),
                    Color::srgb(0.55, 0.78, 0.95),
                    13.0,
                );
            }
            if !step.goal.is_empty() {
                line(p, step.goal.clone(), Color::srgb(0.62, 0.60, 0.58), 11.0);
            }
            for hint in &step.hints {
                line(p, format!("  {hint}"), Color::srgb(0.88, 0.86, 0.83), 12.0);
            }
            if !step.recovery.is_empty() {
                line(p, format!("if not: {}", step.recovery), Color::srgb(0.62, 0.60, 0.58), 11.0);
            }
        });
    }
}

fn line(p: &mut ChildSpawnerCommands, text: String, colour: Color, size: f32) {
    p.spawn((Text::new(text), TextColor(colour), TextFont::from_font_size(size)));
}

/// The overlay's systems, kept together so [`crate::DebuggerPlugin`] adds one thing.
pub struct GuideOverlayPlugin;

impl Plugin for GuideOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Showing>()
            .init_resource::<GuidePlacement>()
            .add_systems(Startup, spawn_guide_overlay)
            .add_systems(Update, (tick_guide, paint_guide).chain());
    }
}
