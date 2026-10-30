# 2026-10-30 ratchet: touched-file check attributed warnings to the wrong file

Focus: none · Outcome: fixes only

## Summary
Host load was 37 on 24 cores, 8 GB free, so the pass was kept to one bug and one clippy run. No consumer or enhancement issue is open (#1, #2 are old user questions about `bevy-debugger-mcp`, not this crate). B-051 was confirmed and fixed.

## Bugs fixed
### B-051: ratchet touched-file check passed files that clippy flags
Must-fix: misleads yes — a gate that said OK while clippy warned.
Root cause: `measure_clippy_files` used `spans[0]`; for `result_large_err` that is the large variant's span in `src/server/error.rs`, not the function. `jq` over one clippy JSON: first-span puts 882 warnings on error.rs and `tools/guide.rs` at 0; primary-span puts 2 on guide.rs and spreads the rest over 87 files.
Fix: select the `is_primary` span in `scripts/ratchet.sh`; `known_red.clippy_by_file` re-attributed from the same measurement (sum 1368, below the 1428 recorded; per-lint and per-feature baselines untouched).
Failing-first check: appended two `result_large_err` fns to guide.rs and measured: old attribution 2 (indistinguishable from clean), primary attribution 8 (> baseline 2, so the gate fails). Reverted afterwards.

Assumed decision: re-attribute the per-file baseline from the current tree instead of the original commit (not recoverable cheaply); totals only go down.

## Evidence
No tool, plugin or server code changed, so no evals or tool tests were re-run. Installed `bevy-fairy` is v0.7.2, the last release.

## Release and install
No release: nothing packaged changed, and the last tag (v0.7.2) was under 6 h old.

## Deferred / open
- Touched-file check still compares to the per-file baseline, not to zero as the command text says; B-020 pays that down.
- B-010 looks stale (input/watch exist); verify and close next pass.

## Next focus
B-010 verification, then B-052 (box `Error`) as its own minor release.
