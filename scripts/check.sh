#!/usr/bin/env bash
# The one command to run before committing. Stops at the first failing step
# and says how to fix it.
#
#   scripts/check.sh            fmt, guards, clippy, all workspace tests
#   scripts/check.sh --quick    fmt, guards, clippy, core + docs tests (pre-commit hook)
#   scripts/check.sh --release  also build the release binary and check its size
#   scripts/check.sh --e2e      also run the window tests (opens windows briefly)
#   scripts/check.sh --bless    accept intentional changes: rewrite the guard
#                               snapshots (add --release to bless the binary size)
#   --profile NAME              build the tests with a cargo profile (CI uses
#                               `ci`: much faster to compile from scratch)
#   --no-clippy                 skip clippy (CI runs it as a parallel job)
#
# Works from any directory. Respects CARGO_BUILD_JOBS etc. from the environment.
set -uo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$MDVIEW_ROOT" || exit 1

quick=0 bless=0 release=0 e2e=0 clippy=1 profile=()
while [[ $# -gt 0 ]]; do
    a=$1
    case $a in
    --profile)
        profile=(--profile "${2:?--profile needs a name}")
        shift
        ;;
    --no-clippy) clippy=0 ;;
    --quick) quick=1 ;;
    --bless) bless=1 ;;
    --release) release=1 ;;
    --e2e) e2e=1 ;;
    -h | --help)
        sed -n '2,/^set /{/^set /d;s/^# \{0,1\}//;p}' "${BASH_SOURCE[0]}"
        exit 0
        ;;
    *) die "unknown option: $a (see scripts/check.sh --help)" ;;
    esac
    shift
done

GUARDS=scripts/guards
start_all=$SECONDS

# run_step <name> <how to fix> <command...>
run_step() {
    local name=$1 fix=$2
    shift 2
    local t0=$SECONDS
    printf '%s==>%s %s%s%s\n' "$C_BOLD" "$C_OFF" "$C_BOLD" "$name" "$C_OFF"
    if "$@"; then
        printf '%s    %s passed (%ss)%s\n' "$C_DIM" "$name" $((SECONDS - t0)) "$C_OFF"
        return 0
    fi
    echo
    err "check failed at step: $name"
    printf '%s\n' "$fix" | sed 's/^/  /' >&2
    exit 1
}

if [[ $bless == 1 ]]; then
    info "Blessing guard snapshots. Review the diff and commit it with a reason."
    "$GUARDS/runtime-deps.sh" --bless || exit 1
    "$GUARDS/gpui-patch.sh" --bless || exit 1
    if [[ $release == 1 ]]; then
        "$GUARDS/binary-size.sh" --build --bless || exit 1
    else
        note "binary size not blessed (needs a fresh release build: add --release)"
    fi
    git --no-pager diff --stat -- tests/guards patches
    info "Dependency or size changes need measured numbers in the commit message:"
    info "  scripts/startup-bench.sh --ref origin/main"
    exit 0
fi

command -v cargo >/dev/null || die "cargo not found"
cargo fmt --version >/dev/null 2>&1 || die "rustfmt missing: rustup component add rustfmt"
cargo clippy --version >/dev/null 2>&1 || die "clippy missing: rustup component add clippy"

# Cheapest first, so the common mistakes fail in seconds.
run_step "rustfmt" "Run: cargo fmt --all" \
    cargo fmt --all --check

run_step "guard: runtime dependencies" \
    "Dependency changes need measured numbers (docs/contributing.md#performance-rules).
After measuring, bless: scripts/check.sh --bless" \
    "$GUARDS/runtime-deps.sh"

run_step "guard: GPUI patch" \
    "Follow docs/gpui-fork.md: mark the change with // mdview:, describe it in
crates/gpui/MDVIEW.md, then regenerate the patch: scripts/check.sh --bless" \
    "$GUARDS/gpui-patch.sh"

if [[ $release == 1 ]]; then
    run_step "guard: release binary size" \
        "See the message above. Bless an intentional, measured change:
scripts/check.sh --release --bless" \
        "$GUARDS/binary-size.sh" --build
fi

if [[ $clippy == 1 ]]; then
    run_step "clippy" "Fix the warnings above (some can be fixed with: cargo clippy --fix --workspace --allow-dirty)" \
        cargo clippy --locked --workspace --all-targets --quiet "${profile[@]}" -- -D warnings
fi

[[ $e2e == 1 ]] && export MDVIEW_E2E=1
if [[ $quick == 1 ]]; then
    run_step "tests: mdview-core" "A parser test failed; see above." \
        cargo test --locked --quiet "${profile[@]}" -p mdview-core
    run_step "tests: docs in sync" "Fix the docs (or the code) as the messages above say." \
        cargo test --locked --quiet "${profile[@]}" -p mdview --test docs_sync
else
    run_step "tests: workspace" "A test failed; see above. Run one test file with
cargo test --test <name>, e.g. cargo test --test cli." \
        cargo test --locked --quiet "${profile[@]}" --workspace
    if [[ $release == 0 ]]; then
        "$GUARDS/binary-size.sh" || exit 1 # only acts on a fresh release build
    fi
fi

ok "all checks passed in $((SECONDS - start_all))s$([[ $quick == 1 ]] && echo ' (quick)')"
