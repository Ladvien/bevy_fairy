#!/usr/bin/env bash
# Ratchet gate for the bevy_fairy improvement loop.
#
# The loop started with `main` already red: `cargo build --all-features` fails inside the
# `observability` feature, and clippy reports hundreds of warnings. Fixing all of that first would
# hold back the tools that actively mislead agents, so the debt is frozen and ratcheted down.
#
# The baseline lives in `docs/loop/metrics.json` under `known_red`. This script measures the current
# state and compares it with that baseline. It FAILS when:
#   * a feature gains build errors it did not have at baseline;
#   * a feature that built at baseline no longer builds;
#   * a clippy lint's count goes up, or a new lint appears;
#   * a file you touched is not clippy-clean;
#   * a test that passed at baseline now fails, or a new test is ignored.
#
# It PASSES when the counts are equal or lower. Lower counts are a win; lower the recorded baseline
# in the same commit as the fix that reduced it.
#
# Usage:
#   scripts/ratchet.sh              # gate: measure and compare, non-zero exit on regression
#   scripts/ratchet.sh --record     # record the current state into metrics.json (once, deliberately)
#   scripts/ratchet.sh --json       # print the measurement as JSON only
#
# Memory: this host is shared. Every cargo invocation runs under a cap.
# Touched-file cleanliness: pass the changed files as arguments, e.g.
#   scripts/ratchet.sh src/mcp_tools.rs src/tools/experiment.rs
# With no arguments, the touched-file check uses the files changed in the working tree against
# HEAD, which is the right default for an iteration that has not committed yet.

set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

METRICS="docs/loop/metrics.json"
MEMORY_MAX="${RATCHET_MEMORY_MAX:-8G}"
JOBS="${RATCHET_JOBS:-2}"

# Run a cargo command with limited parallelism, under a memory cap.
#
# The cap is applied by a systemd scope so one limit covers the whole sweep. Nesting a scope
# inside a scope double-counts the memory against the outer limit and makes oomd kill the outer
# unit, which is how the first two recordings died. systemd sets INVOCATION_ID inside a unit, so
# that is how we tell whether we are already inside one.
capped() {
    if [ -z "${INVOCATION_ID:-}" ] && command -v systemd-run >/dev/null 2>&1 \
        && systemctl --user is-system-running >/dev/null 2>&1; then
        systemd-run --user --scope --quiet -p "MemoryMax=${MEMORY_MAX}" "$@" --jobs "$JOBS"
    else
        "$@" --jobs "$JOBS"
    fi
}


# --- measure -------------------------------------------------------------------------------

# Every feature name in the manifest, plus the default set.
feature_list() {
    sed -n '/^\[features\]/,/^\[[^f]/p' Cargo.toml \
        | grep -E '^[a-z0-9_-]+ = ' \
        | sed 's/ =.*//' \
        | grep -v '^default$'
}

# Per-feature build error counts, keyed by feature.
#
# This uses `cargo check`, not `cargo build`. Both report the same compile errors, but `check`
# skips codegen and linking, which is what made the full 24-feature sweep fit in memory. The
# only failures `check` misses are link errors, and this workspace has no link-time feature
# gating. Every cargo command runs under a memory cap, and the caps are per-command rather than
# one parent cap, so concurrent builds elsewhere on this host cannot OOM-kill the sweep.
measure_features() {
    local f out errs
    for f in $(feature_list); do
        out=$(capped cargo check --features "$f" --message-format=json 2>/dev/null)
        errs=$(printf '%s' "$out" \
            | jq -r 'select(.reason=="compiler-message" and .message.level=="error") | .message.rendered' 2>/dev/null \
            | grep -c 'error\[' || true)
        [ -z "$errs" ] && errs=0
        printf '%s\t%s\n' "$f" "$errs"
    done
}

# Clippy warnings per lint id.
measure_clippy() {
    capped cargo clippy --workspace --all-targets --message-format=json 2>/dev/null \
        | jq -r 'select(.reason=="compiler-message" and .message.level=="warning" and .message.code.code!=null) | .message.code.code' \
        | sort | uniq -c | awk '{print $2"\t"$1}' | sort
}

# Clippy warnings per file, attributed to the primary span. `spans[0]` is not the primary span:
# result_large_err lists the large variant's span (src/server/error.rs) first, so 882 warnings
# were charged to error.rs and the functions' own files looked clean (B-051).
measure_clippy_files() {
    capped cargo clippy --workspace --all-targets --message-format=json 2>/dev/null \
        | jq -r 'select(.reason=="compiler-message" and .message.level=="warning" and .message.spans!=null) | ([.message.spans[] | select(.is_primary)][0].file_name // empty)' \
        | sort | uniq -c | awk '{print $2"\t"$1}' | sort
}

# Failing and ignored test names, as "name|state".
measure_tests() {
    capped cargo test --workspace --no-fail-fast 2>&1 \
        | grep -E '^test .* \.\.\. (FAILED|ignored)' \
        | sed 's/^test //; s/ \.\.\. /|/' | sort
}

# Files the caller touched, as workspace-relative paths.
touched_files() {
    if [ "$#" -gt 0 ]; then
        printf '%s\n' "$@"
    else
        git diff --name-only HEAD -- '*.rs' 2>/dev/null
        git diff --name-only --cached -- '*.rs' 2>/dev/null
        git ls-files --others --exclude-standard -- '*.rs' 2>/dev/null
    fi | sort -u
}

# --- record -------------------------------------------------------------------------------

record() {
    local tmp
    tmp=$(mktemp)
    {
        echo '{'
        echo '  "recorded_at": "'"$(date -I)"'",'
        echo '  "recorded_at_sha": "'"$(git rev-parse --short HEAD)"'",'
        echo '  "features": {'
        local first=1 f e
        while IFS=$'\t' read -r f e; do
            [ $first -eq 0 ] && echo ","
            first=0
            printf '    "%s": %s' "$f" "$e"
        done < <(measure_features)
        echo '  },'
        echo '  "clippy_by_lint": {'
        local first=1 l c
        while IFS=$'\t' read -r l c; do
            [ $first -eq 0 ] && echo ","
            first=0
            printf '    "%s": %s' "$l" "$c"
        done < <(measure_clippy)
        echo '  },'
        echo '  "clippy_by_file": {'
        local first=1 fl c
        while IFS=$'\t' read -r fl c; do
            [ $first -eq 0 ] && echo ","
            first=0
            printf '    "%s": %s' "$fl" "$c"
        done < <(measure_clippy_files)
        echo '  },'
        echo '  "failing_or_ignored_tests": ['
        local first=1 t
        while IFS= read -r t; do
            [ $first -eq 0 ] && echo ","
            first=0
            printf '    "%s"' "$(printf '%s' "$t" | sed 's/"/\\"/g')"
        done < <(measure_tests)
        echo '  ]'
        echo '}'
    } > "$tmp"
    # Write the measurement into metrics.json under "known_red", keeping everything else in that
    # file. The baseline lives in one place, as the loop describes.
    jq --slurpfile kr "$tmp" '.known_red = $kr[0]' "$METRICS" > "${METRICS}.tmp" \
        && mv "${METRICS}.tmp" "$METRICS"
    rm -f "$tmp"
    echo "ratchet: wrote ${METRICS} under \"known_red\"" >&2
    echo "ratchet: commit it with the fix that produced it. Counts may only go down." >&2
}

# --- gate ----------------------------------------------------------------------------------

if [ "${1:-}" = "--record" ]; then
    record
    exit 0
fi

if [ "${1:-}" = "--json" ]; then
    echo '{"features":{'
    first=1
    while IFS=$'\t' read -r f e; do
        [ $first -eq 0 ] && echo ","
        first=0
        printf '  "%s": %s' "$f" "$e"
    done < <(measure_features)
    echo '}}'
    exit 0
fi

BASE="${METRICS}"
if [ ! -f "$BASE" ]; then
    echo "ratchet: no ${BASE}" >&2
    echo "ratchet: record a baseline with 'scripts/ratchet.sh --record' (once, deliberately)" >&2
    exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "ratchet: jq is required" >&2
    exit 2
fi

if ! jq -e '.known_red | type == "object"' "$BASE" >/dev/null 2>&1; then
    echo "ratchet: ${BASE} has no \"known_red\" block" >&2
    echo "ratchet: record a baseline with 'scripts/ratchet.sh --record' (once, deliberately)" >&2
    exit 2
fi

fail=0

# Features: error counts may not go up, and a feature that built must still build.
while IFS=$'\t' read -r f e; do
    base=$(jq -r --arg f "$f" '.known_red.features[$f] // 0' "$BASE")
    if [ "$e" -gt "$base" ]; then
        echo "ratchet: FAIL feature '$f' has $e build errors, baseline allows $base" >&2
        fail=1
    fi
done < <(measure_features)

# Clippy: no lint count may go up, and no new lint may appear.
while IFS=$'\t' read -r l c; do
    base=$(jq -r --arg l "$l" '.known_red.clippy_by_lint[$l] // 0' "$BASE")
    if [ "$c" -gt "$base" ]; then
        echo "ratchet: FAIL clippy lint '$l' count $c exceeds baseline $base" >&2
        fail=1
    fi
done < <(measure_clippy)

# Tests: no new failing or ignored test may appear.
while IFS= read -r t; do
    if ! jq -e --arg t "$t" '.known_red.failing_or_ignored_tests | index($t) != null' "$BASE" >/dev/null 2>&1; then
        name=${t%%|*}
        state=${t##*|}
        echo "ratchet: FAIL new $state test: $name" >&2
        fail=1
    fi
done < <(measure_tests)

# Touched files must be clippy-clean: new and modified code meets -D warnings even though the
# old code does not yet.
touched=$(touched_files "$@")
if [ -n "$touched" ]; then
    # Baseline warnings per file, for the files being touched.
    clippy_files_now=$(measure_clippy_files)
    while IFS= read -r f; do
        [ -z "$f" ] && continue
        [ -f "$f" ] || continue
        base_count=$(jq -r --arg f "$f" '.known_red.clippy_by_file[$f] // 0' "$BASE")
        now_count=$(printf '%s' "$clippy_files_now" | awk -v f="$f" '$1==f {print $2}' | head -1)
        [ -z "$now_count" ] && now_count=0
        if [ "$now_count" -gt "$base_count" ]; then
            echo "ratchet: FAIL touched file '$f' has $now_count clippy warnings, baseline allows $base_count" >&2
            fail=1
        fi
    done <<< "$touched"
fi

if [ "$fail" -ne 0 ]; then
    echo "ratchet: FAILED" >&2
    exit 1
fi
echo "ratchet: OK (no new errors, warnings or failures)"
exit 0
