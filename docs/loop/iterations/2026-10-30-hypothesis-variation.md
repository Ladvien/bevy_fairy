# 2026-10-30 hypothesis-variation: advertised variation_type values now match what the handler runs

Focus: honesty audit · Commits: c5373bd..HEAD · Outcome: fixes only

## Summary
`hypothesis` advertised `variation_type` values `parameter` and `scenario` (plus `parameter_name`/`parameter_values`); the handler refused the first two ("Unknown variation type") and never read the fields, while the strategies it runs (`grid`, `boundary`, `fuzz`) could not be requested. The schema now is an enum of what runs, with each strategy's fields advertised. `stress_test` no longer overwrites caller's iterations/variation/mutation args.

## Bugs fixed
### B-060: hypothesis schema offers variation types the handler refuses
Must-fix: misleads yes — advertised values errored; fields ignored.
Fix: `VariationType` enum in `src/server/mcp_tools.rs`, fields added, `parameter_*` removed; `handle_stress_test` fills defaults only where absent. Failing-first check: `tests/tool_contract.rs::every_advertised_variation_type_runs` (failed: no enum in schema; passes now).

## Evidence
Gate: `scripts/ratchet.sh` OK; `cargo test --workspace --all-features --no-fail-fast` all green; docs/mcp-tools.md regenerated.
Not run: dev/held-out evals (no eval task touches hypothesis), mutation testing, live MCP client pass. Reported as not done.

## Test and eval diffs
Added one test in tests/tool_contract.rs. CHANGELOG.md line naming `parameter_name`/`parameter_values` in the old migration table corrected (docs_drift requires it name real fields); no assertion changed.

## Release and install
No release: v0.7.2 is ~2 h old (6 h rule); change rides in 0.8.0 (breaking: removed fields). Installed binary not rebuilt this pass.

## Deferred / open
- `replay`, `scene`, `observe` fields (`detailed`, `diff`, `reflection`) have empty schema descriptions; `replay.action` unenumerated.
- B-021 burst capture, B-024.

## Next focus
Document/enumerate replay, scene, observe args (tool contract), then B-021.
