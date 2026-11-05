#!/usr/bin/env bash
# Mutation testing, locally only. It changes the code (e.g. `<` to `<=`) and
# checks that some test fails; a mutant that survives is logic no test checks.
#
#   scripts/mutants.sh               mutate only the lines this branch changed
#                                    (committed, since the merge base with
#                                    origin/main): seconds to a few minutes
#   scripts/mutants.sh --base REF    ... since the merge base with REF instead
#   scripts/mutants.sh --all         every mutant in mdview-core plus the app's
#                                    pure modules: about an hour, for rare audits
#   scripts/mutants.sh ... -- ARGS   extra arguments for cargo mutants
#
# Runs in a scratch worktree under ${XDG_CACHE_HOME:-~/.cache}/mdview-mutants,
# reset to HEAD each time, so your working tree is never touched and the build
# is reused between runs. Each process is capped with ulimit -v, because some
# mutants make a loop allocate without end. Equivalent mutants are excluded in
# .cargo/mutants.toml. Exits 0 when no mutant survives.
set -uo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$MDVIEW_ROOT" || exit 1

base=origin/main all=0 extra=()
while [[ $# -gt 0 ]]; do
    case $1 in
    --base)
        base=${2:?--base needs a git ref}
        shift
        ;;
    --all) all=1 ;;
    --)
        shift
        extra=("$@")
        break
        ;;
    -h | --help)
        sed -n '2,/^set /{/^set /d;s/^# \{0,1\}//;p}' "${BASH_SOURCE[0]}"
        exit 0
        ;;
    *) die "unknown option: $1 (see scripts/mutants.sh --help)" ;;
    esac
    shift
done

command -v cargo-mutants >/dev/null || die "cargo-mutants missing: cargo install --locked cargo-mutants"
[[ -z $(git status --porcelain --untracked-files=no) ]] ||
    note "uncommitted changes are not tested: the run uses HEAD ($(git rev-parse --short HEAD))"

cache=${XDG_CACHE_HOME:-$HOME/.cache}/mdview-mutants
wt=$cache/wt out=$MDVIEW_ROOT/target/mutants
mkdir -p "$cache"
if [[ ! -d $wt ]]; then
    git worktree add --quiet --detach "$wt" HEAD || die "can't create $wt"
fi
# --in-place below edits $wt; an interrupted run can leave a mutation behind,
# so always start from a clean HEAD.
git -C "$wt" checkout --quiet --force --detach "$(git rev-parse HEAD)" &&
    git -C "$wt" clean --quiet -fd || die "can't reset $wt"

args=(--in-place)
if [[ $all == 1 ]]; then
    info "Full run (about an hour). Core crate first, then the app's pure modules."
    runs=(
        "-p mdview-core --test-package mdview-core --timeout 30"
        "-p mdview -f src/tabs.rs -f src/links.rs -f src/cli.rs --timeout 60"
    )
else
    git rev-parse --verify --quiet "$base" >/dev/null || base=main
    mb=$(git merge-base "$base" HEAD) || die "no merge base with $base"
    diff_file=$cache/changes.diff
    git diff "$mb" HEAD -- '*.rs' >"$diff_file"
    if [[ ! -s $diff_file ]]; then
        ok "no Rust changes since $(git rev-parse --short "$mb") ($base): nothing to mutate"
        exit 0
    fi
    args+=(--in-diff "$diff_file")
    # The core crate builds in seconds; only pay for GPUI when src/ changed.
    if git diff --quiet "$mb" HEAD -- src; then
        runs=("-p mdview-core --test-package mdview-core --timeout 30")
    else
        runs=("--workspace --timeout 60")
    fi
    info "Mutating lines changed since $(git rev-parse --short "$mb") ($base)."
fi

status=0 n=0
rm -rf "$out"
for run in "${runs[@]}"; do
    n=$((n + 1))
    mkdir -p "$out/$n"
    # 2.5 GB per process is plenty for the core crate; compiling GPUI needs more.
    limit=2500000
    [[ $run == *mdview-core* ]] || limit=6000000
    # shellcheck disable=SC2086 # $run is a list of arguments
    (
        cd "$wt" && ulimit -v $limit &&
            CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
                nice cargo mutants $run "${args[@]}" --output "$out/$n" "${extra[@]}"
    )
    s=$?
    # 2 = some mutants survived, 3 = some timed out (they count as caught).
    [[ $s == 0 || $s == 3 ]] || status=1
done
git -C "$wt" checkout --quiet --force HEAD

if [[ $status == 0 ]]; then
    ok "every mutant was caught (details: $out/*/mutants.out)"
else
    err "some mutants survived, or cargo mutants failed:"
    cat "$out"/*/mutants.out/missed.txt 2>/dev/null | sed 's/^/  /' >&2
    info "Add a test that fails for each, or, if one is truly equivalent (no"
    info "observable change), exclude it in .cargo/mutants.toml with the reason."
fi
exit $status
