# 2026-10-30 service-docs: stale service/subcommand docs removed

Focus: none · Outcome: fixes only (B-031) · No release: v0.7.2 tagged 4 minutes before this pass, 6-hour rule; changes ride in the next release.

## Bugs fixed
### B-031: docs and files told readers to run subcommands that do not exist
Must-fix: misleads y, blocks y (the plist exited 2 on load) — launchd plist ran `serve --config`; troubleshooting ran `export-sessions`; Homebrew formula ran `setup-claude`/`serve` and asserted `doctor` in `--help`.
Fix: deleted `launchd/`, `book/MACOS_SERVICE.md` (an MCP stdio server is spawned by its client); corrected formula, `scripts/install.sh`, troubleshooting; dropped `launchd/*` from Cargo exclude.
Failing-first check: `tests/docs_drift.rs::no_shipped_file_tells_the_reader_to_run_a_subcommand_the_binary_lacks`.

## Evidence
Tests: docs_drift 9/9, cli_args 4/4, tool_list 5/5. Evals not run: no tool change. Metrics unchanged (no Rust source change).

## Test and eval diffs
Added one test to tests/docs_drift.rs; none weakened.

## Deferred / open
Issues #1, #2 are non-actionable (stale/question). Host load average was ~26 during this pass.

## Next focus
B-010/B-021 (burst capture), or B-052 Error boxing.
