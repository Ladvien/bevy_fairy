# bevy_debugger_mcp loop: literature critique

**Date:** 2026-10-21
**Purpose:** The evidence behind `.omp/commands/build-loop.md` (named `bevy-debugger-loop.md` until 2026-10-28).
**Method:**
- 16 concepts specific to this loop were checked against the home-still corpus by three parallel searches (clusters H, I and J below).
- Concepts shared with the bevy_grimoire loop reuse that critique (`2026-10-21-bevy-emerge-loop-literature-critique.md`): agent loop design, verification layering, reviewer bias, context degradation, test gaming, and frame-sequence capture.
- Web research covered the crate's published state, the MCP 2026-07-28 spec, and Anthropic's tool-evaluation guidance.
- Every stem cited below was confirmed with `catalog_read`.

**Input review:** The crate review supplied with the request found:
- four tools whose argument names don't match their handlers;
- two tools that fabricate data;
- a CLAUDE.md that is wrong about screen capture;
- an injected cursor that probably can't trigger picking;
- about 60 unreachable modules;
- single-frame capture only.

## Verdicts

### H. Tool correctness, honesty and contracts

|ID|Concept|Verdict|Rule it produced|
|---|---|---|---|
|H1|Schema/implementation drift|Thin|One source of truth: the argument struct generates the schema, the handler deserialises the same struct, and unknown fields are rejected. A contract test sends every advertised field|
|H2|Fabricated or placeholder output|Thin, but the concern is supported indirectly|No constants standing in for measurements. Return a refusal or `unavailable: reason`. Every output needs a ground-truth test against the live game|
|H3|Test oracle problem|**Supported**|Passing tests are not evidence. Use ground-truth fixtures, metamorphic checks and mutation testing on handlers|
|H4|Fail loudly|Supported (practitioner books)|Unknown values return a structured error listing the valid values with an example. Never fall through to a default|
|H5|Docs that contradict the code|**Supported** for human readers|Generate docs from the schemas. A test fails when docs name things that don't exist. Delete stale text, don't only append|

### I. Capabilities that help agents debug games

|ID|Concept|Verdict|Rule it produced|
|---|---|---|---|
|I1|Pixels plus state|Supported for GUI agents, thin for games|Pair every capture with a compact state snapshot from the same frame|
|I2|Fidelity of injected input|Thin|Inject through the same path as real input. Confirm a state change afterwards. Report "input not delivered" separately from game bugs|
|I3|Record and replay|Supported, with caveats|Prefer an always-on ring buffer of state and frames that dumps on a trigger. Don't promise exact input replay on a non-deterministic engine|
|I4|Hypothesis-driven debugging|**Mixed**|Keep "predict, run, check" only with machine-checkable predicates and a "predicate never evaluated" flag. Otherwise delete|
|I5|Continuous observation over time|**Supported**|Watch tools and frame sequences outrank new one-shot queries|
|I6|Reliability of profiling and anomaly detection|Mixed; false positives are the central issue|Report a baseline, repeat count, confidence and the tool's own overhead. An anomaly alone is never a verdict|

### J. Tool surface, maintainability and evaluation

|ID|Concept|Verdict|Rule it produced|
|---|---|---|---|
|J1|Tool count and overlap vs selection accuracy|Thin in the corpus; RAG-MCP now added|Merge overlapping tools. Remove broken ones. No "prefer this tool" wording|
|J2|Dead code and complexity|**Supported**|Deleting and consolidating code are valid changes. Ratchet lines of code, dependency count, warnings and tool count|
|J3|Evaluating tools with agents|Supported|Fixed task suite plus held-out tasks. Track success, tool calls, invalid-call rate and loops. No regressions allowed|
|J4|Small changes vs large agent refactors|Supported (mostly indirect)|One change per iteration with a diff cap. Delete module by module|
|J5|Protocol and dependency upgrades|Thin|Make the rmcp/protocol upgrade its own isolated iteration, gated by evals and tested against a current client|

## The evidence that shaped the loop most

1. **Passing tests prove little (H3).** The oracle survey is explicit that automating inputs does not decide whether outputs are correct (10.1109/tse.2014.2372785). Faulty tests that pass wrong code make an agent declare a task done too early (10.48550/arXiv.2409.02977). That fits the review, where 463 tests passed around the broken tools. So every fix in this loop starts with a test that fails against ground truth from a fixture game.
2. **Agents invent values rather than report a gap (H2).** In ToolSandbox, GPT-4 invented a current timestamp instead of reporting the missing information (10.48550/arXiv.2408.04682). Self-correction is weak without reliable external feedback (10.48550/arXiv.2310.01798). A debugging tool that fabricates gives the calling agent false evidence, so fabrication is treated as the most serious class of bug.
3. **Temporal evidence (I5).** Comparing against a reference frame consistently improved glitch detection across five VLMs (10.48550/arXiv.2604.11082). Temporal-logic monitors over game-loop events catch failures that single snapshots miss (10.1145_2700529). This supports watch tools and a ring buffer over new one-shot queries.
4. **Hypothesis tooling (I4).** AutoSD-style scientific debugging takes about 5× longer. It fails when it proposes a breakpoint that is never hit and then blames the test instead of revising the hypothesis (as summarised in 10.48550/arXiv.2409.02977). SWE-agent shows that interface design matters more than feature count (10.48550/arXiv.2405.15793). So the hypothesis and experiment tools must make predicates machine-checkable, or be deleted.
5. **Delete before adding (J2).** Across 1,899 MCP servers, 66% had critical code smells, with cognitive complexity the most common, and smaller, actively maintained servers fared better (10.1145/3814959). Smelly classes are more change-prone and fault-prone (10.1007/s10664-017-9535-z). After Cursor adoption, complexity rose persistently and slowed later velocity (10.48550/arXiv.2511.04427).
6. **Agent context files decay (H5).** CLAUDE.md and AGENTS.md files grow through additions and almost never through deletions (10.1145/3840295). Outdated code references are common across GitHub projects (10.1007/s10664-023-10397-6). This crate's CLAUDE.md is already wrong about capture.
7. **Eval-driven tool development.** Anthropic's guidance:
   - realistic tasks that need many tool calls, each with a verifiable outcome;
   - tracking accuracy, tool calls, tokens and errors;
   - reading transcripts, where many invalid-parameter errors point to unclear descriptions;
   - held-out sets to catch overfitting.

## Facts from the web that the loop depends on

- **The crate is published.** bevy_debugger_mcp 0.1.8 has been on crates.io since 2026-09-18 (repo: ladvien/bevy_debugger_mcp). docs.rs doesn't mention bevy_debugger_bevy, so the plugin crate may not be published yet. The loop treats a first publish of a new crate name as the human's decision.
- **The protocol is far behind.** The crate uses MCP protocol 2024-11-05. The current spec, 2026-07-28, changes several things:
  - it is stateless: the initialize handshake is gone and each request carries its version in `_meta`;
  - it adds a required `server/discover` method;
  - long-running work moves to a tasks extension that clients poll with `tasks/get`;
  - progress notifications travel on the stream of the request they belong to;
  - `tools/list` should be returned in a deterministic order;
  - logging is deprecated in favour of stderr or OpenTelemetry.

  The current rmcp SDK implements 2026-07-28. Any streaming "watch" design has to fit this model.
- **Related project.** bevy_brp_mcp (natepiano) covers generic BRP operations, watch, screenshots and keyboard input. It is archived; development moved to the bevy_brp workspace. This crate's distinct value is honest, evidence-producing debugging tools, not generic BRP coverage.

## Acquisition list

**Downloaded this session** (conversion left to run on its own schedule):
- RAG-MCP: Mitigating Prompt Bloat in LLM Tool Selection (2025), 10.48550/arXiv.2505.03275
- MCP-Universe: Benchmarking LLMs with Real-World MCP Servers (2025), 10.48550/arXiv.2508.14704
- Kang et al., Explainable Automated Debugging via LLM-driven Scientific Debugging / AutoSD (2023), 10.48550/arXiv.2304.02195
- Sun et al., Tools Fail: Detecting Silent Errors in Faulty Tools (2024), 10.48550/arXiv.2406.19228

**Not downloaded. Open-access status unknown:**
- Coverage is not strongly correlated with test suite effectiveness (2014), 10.1145/2568225.2568271
- Assertions are strongly correlated with test suite effectiveness (2015), 10.1145/2786805.2786858
- Record and Replay for Android: Are We There Yet in Industrial Cases? (2017), 10.1145/3106237.3117769

**Infrastructure:** home-still's external providers were again partly rate-limited, and CORE still has no API key.

## Sources (confirmed in the home-still catalog)

|Stem|Title|
|---|---|
|10.1109_tse.2014.2372785|The Oracle Problem in Software Testing: A Survey (2014)|
|10.48550_arxiv.2408.04682|ToolSandbox (2024)|
|10.48550_arXiv.2310.01798|LLMs Cannot Self-Correct Reasoning Yet (2023)|
|10.48550_arxiv.2409.02977|LLM-Based Agents for Software Engineering: A Survey (2024)|
|10.1007_s10664-023-10397-6|Detecting outdated code element references in documentation (2023)|
|10.1145_3840295|Agent READMEs: Context Files for Agentic Coding (2026)|
|10.1007_s41019-025-00296-9|LLM-Based Agents for Tool Learning: A Survey (2025)|
|10.48550_arxiv.2404.07972|OSWorld (2024)|
|10.1109_cog51982.2022.9893630|Inspector (2022)|
|10.48550_arxiv.2605.13918|CA2: Code-Aware Agent for Automated Game Testing (2026)|
|10.1145_2700529|Automated bug finding in video games: runtime monitoring (2017, title not in catalog)|
|gameaipro3-ch06-debugging-ai-instant-game-scrubbing|Game AI Pro 3, ch. 6, Instant In-Game Scrubbing|
|10.48550_arxiv.2604.11082|RESP (2026)|
|10.48550_arxiv.2604.07818|GLIDE / VideoGlitchBench (2026)|
|10.48550_arxiv.2604.04047|Software Testing Beyond Closed Worlds (2026)|
|10.48550_arxiv.2601.16881|Selective Instrumentation in Large C++ Game Engines (2026)|
|10.1609_aiide.v16i1.7409|DCNNs to Detect Rendered Glitches (2020)|
|10.48550_arxiv.2405.15793|SWE-agent (2024)|
|10.48550_arxiv.2407.01489|Agentless (2024)|
|10.1145_3814959|MCP at First Glance (2026)|
|10.48550_arxiv.2503.23278|MCP: Landscape, Security Threats (2025)|
|10.1007_s10664-017-9535-z|Diffuseness and impact of code smells (2017)|
|10.48550_arxiv.2511.04427|Speed at the Cost of Quality (Cursor) (2025)|
|10.1145_3772318.3791642|From Junior to Senior (2026)|
|10.48550_arxiv.2601.01743|AI Agent Systems: Architectures, Applications, and Evaluation (2026)|

**Web sources:**
- [docs.rs: bevy_debugger_mcp](https://docs.rs/bevy_debugger_mcp)
- [MCP 2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
- [Anthropic: Writing effective tools for agents](https://anthropic.com/engineering/writing-tools-for-agents)
- [bevy_brp_mcp](https://mcpservers.org/servers/natepiano/bevy_brp_mcp)
