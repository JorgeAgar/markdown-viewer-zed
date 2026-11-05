#!/usr/bin/env bash
# A/B startup benchmark. Prints a summary you can paste into a commit message.
#
#   scripts/startup-bench.sh OLD_BIN NEW_BIN [options]
#   scripts/startup-bench.sh --ref REF [options]     REF's release build vs ./target/release/mdview
#
# Options:
#   -r, --runs N       first-frame runs per binary (default 15)
#   --help-runs N      `mdview --help` runs per binary, for the warm-path cost (default 200)
#   -f, --file FILE    document to open (default examples/demo.md)
#   --budget PCT       exit 1 if NEW's median first frame is more than PCT% slower
#                      than OLD's (for local use; shared CI machines are too noisy)
#
# Cold start: runs `MDVIEW_TRACE_STARTUP=quit BIN -n FILE` (opens a window for a
# moment), alternating OLD/NEW and swapping the order every round so drift
# (thermal, caches) hits both equally. One unmeasured warm-up run each first.
#
# --ref builds REF in a git worktree under ${XDG_CACHE_HOME:-~/.cache}/mdview-bench
# (outside the repo, kept between runs so rebuilds are incremental) and
# rebuilds ./target/release/mdview if it's out of date.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() {
    sed -n '2,/^set /{/^set /d;s/^# \{0,1\}//;p}' "${BASH_SOURCE[0]}"
    exit "${1:-0}"
}

runs=15 help_runs=200 file=$MDVIEW_ROOT/examples/demo.md budget="" ref="" bins=()
while (($#)); do
    case $1 in
    -r | --runs) runs=$2 && shift ;;
    --help-runs) help_runs=$2 && shift ;;
    -f | --file) file=$(realpath "$2") && shift ;;
    --budget) budget=$2 && shift ;;
    --ref) ref=$2 && shift ;;
    -h | --help) usage ;;
    -*) err "unknown option: $1" && usage 1 ;;
    *) bins+=("$1") ;;
    esac
    shift
done
[[ $runs =~ ^[1-9][0-9]*$ && $help_runs =~ ^[1-9][0-9]*$ ]] || die "runs must be positive integers"
[[ -z $budget || $budget =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "--budget takes a percentage, e.g. --budget 10"
[[ -f $file ]] || die "no such file: $file"

build_ref() {
    local ref=$1 cache=${XDG_CACHE_HOME:-$HOME/.cache}/mdview-bench
    local sha
    sha=$(git -C "$MDVIEW_ROOT" rev-parse --verify --quiet "$ref^{commit}") || die "unknown git ref: $ref"
    git -C "$MDVIEW_ROOT" cat-file -e "$sha:Cargo.toml" 2>/dev/null ||
        die "$ref ($sha) has no Cargo.toml: not an mdview commit? (local 'main' may be stale: try origin/main)"
    mkdir -p "$cache/bin"
    OLD_BIN=$cache/bin/mdview-$sha
    OLD_LABEL="$ref (${sha:0:9})"
    [[ -x $OLD_BIN ]] && return 0

    local wt=$cache/src
    exec 9>"$cache/lock"
    flock 9 || die "couldn't lock $cache"
    git -C "$MDVIEW_ROOT" worktree prune
    if ! git -C "$wt" rev-parse --git-dir >/dev/null 2>&1; then
        rm -rf "$wt"
        git -C "$MDVIEW_ROOT" worktree add --quiet --detach "$wt" "$sha" >&2
    else
        git -C "$wt" checkout --quiet --force --detach "$sha"
        git -C "$wt" clean -fdxq -e target
    fi
    if [[ ! -d $cache/target/release && -d $MDVIEW_ROOT/target/release/deps ]]; then
        # Seed the cache so the registry crates aren't rebuilt (GPUI, a path
        # dependency, still is, once).
        info "seeding $cache/target from ./target/release..." >&2
        mkdir -p "$cache/target"
        cp -a --reflink=auto "$MDVIEW_ROOT/target/release" "$cache/target/"
    fi
    info "building $OLD_LABEL in $wt (the first time compiles GPUI: several minutes)..." >&2
    (cd "$wt" && CARGO_TARGET_DIR=$cache/target cargo build --release --quiet) >&2 ||
        die "release build of $ref failed"
    cp "$cache/target/release/mdview" "$OLD_BIN"
}

if [[ -n $ref ]]; then
    ((${#bins[@]} == 0)) || die "--ref takes no binaries (it compares REF with ./target/release/mdview)"
    build_ref "$ref"
    info "building ./target/release/mdview if it's out of date..." >&2
    (cd "$MDVIEW_ROOT" && cargo build --release --quiet) >&2 || die "release build failed"
    NEW_BIN=$MDVIEW_ROOT/target/release/mdview
    NEW_LABEL="working tree ($(git -C "$MDVIEW_ROOT" rev-parse --short=9 HEAD)$(git -C "$MDVIEW_ROOT" diff --quiet HEAD -- src crates Cargo.toml Cargo.lock || echo '+dirty'))"
else
    ((${#bins[@]} == 2)) || usage 1
    OLD_BIN=$(realpath "${bins[0]}") NEW_BIN=$(realpath "${bins[1]}")
    OLD_LABEL=${bins[0]} NEW_LABEL=${bins[1]}
fi
for b in "$OLD_BIN" "$NEW_BIN"; do [[ -x $b ]] || die "not an executable: $b"; done
[[ -n ${WAYLAND_DISPLAY:-}${DISPLAY:-} ]] || die "no display (WAYLAND_DISPLAY/DISPLAY): the cold-start runs open a window"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# One cold start; appends the first-frame time in ms to $2.
first_frame() {
    local out
    out=$(MDVIEW_TRACE_STARTUP=quit timeout 60 "$1" -n "$file" 2>&1 >/dev/null) ||
        die "$1 -n $file failed:"$'\n'"$out"
    awk '/^first frame: / {
        v = $3; u = v; sub(/[^0-9.]+$/, "", v); sub(/^[0-9.]+/, "", u)
        f = (u == "s") ? 1000 : (u == "ms") ? 1 : (u == "µs" || u == "us") ? 0.001 : (u == "ns") ? 1e-6 : -1
        if (f < 0) exit 1
        printf "%.3f\n", v * f; found = 1 }
        END { exit !found }' <<<"$out" >>"$2" || die "no 'first frame:' line from $1:"$'\n'"$out"
}

# Mean wall time of `BIN --help` in ms over $help_runs runs (process start to
# exit: the cost every warm-path launch pays before doing anything).
help_mean() {
    local t0 t1 i
    t0=$(date +%s%N)
    for ((i = 0; i < help_runs; i++)); do "$1" --help >/dev/null; done
    t1=$(date +%s%N)
    awk -v a="$t0" -v b="$t1" -v n="$help_runs" 'BEGIN { printf "%.3f", (b - a) / n / 1e6 }'
}

info "warming up..." >&2
first_frame "$OLD_BIN" /dev/null
first_frame "$NEW_BIN" /dev/null
info "measuring: $runs cold starts each, alternating..." >&2
for ((i = 0; i < runs; i++)); do
    if ((i % 2 == 0)); then
        first_frame "$OLD_BIN" "$tmp/old" && first_frame "$NEW_BIN" "$tmp/new"
    else
        first_frame "$NEW_BIN" "$tmp/new" && first_frame "$OLD_BIN" "$tmp/old"
    fi
done
info "measuring: $help_runs --help runs each..." >&2
help_mean "$OLD_BIN" >/dev/null # warm the page cache
help_old=$(help_mean "$OLD_BIN") help_new=$(help_mean "$NEW_BIN")
help_new2=$(help_mean "$NEW_BIN") help_old2=$(help_mean "$OLD_BIN")
help_old=$(awk -v a="$help_old" -v b="$help_old2" 'BEGIN { printf "%.3f", (a + b) / 2 }')
help_new=$(awk -v a="$help_new" -v b="$help_new2" 'BEGIN { printf "%.3f", (a + b) / 2 }')

# min median max of a file of numbers
stats() { sort -g "$1" | awk '{ v[NR] = $1 } END {
    m = (NR % 2) ? v[(NR + 1) / 2] : (v[NR / 2] + v[NR / 2 + 1]) / 2
    printf "%s %s %s\n", v[1], m, v[NR] }'; }
read -r old_min old_med old_max < <(stats "$tmp/old")
read -r new_min new_med new_max < <(stats "$tmp/new")
size_old=$(stat -c %s "$OLD_BIN") size_new=$(stat -c %s "$NEW_BIN")

pct() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%+.1f%%", (b - a) * 100 / a }'; }
row() { printf '%-22s %14s %14s %9s\n' "$@"; }
ms() { awk -v v="$1" 'BEGIN { printf "%.1f ms", v }'; }

echo
echo "Startup, $(basename "$file"), $runs cold starts + $help_runs x --help each (scripts/startup-bench.sh):"
echo "  old: $OLD_LABEL"
echo "  new: $NEW_LABEL"
row "" old new change
row "first frame min" "$(ms "$old_min")" "$(ms "$new_min")" "$(pct "$old_min" "$new_min")"
row "first frame median" "$(ms "$old_med")" "$(ms "$new_med")" "$(pct "$old_med" "$new_med")"
row "first frame max" "$(ms "$old_max")" "$(ms "$new_max")" "$(pct "$old_max" "$new_max")"
row "--help mean" "$(awk -v v="$help_old" 'BEGIN { printf "%.2f ms", v }')" \
    "$(awk -v v="$help_new" 'BEGIN { printf "%.2f ms", v }')" "$(pct "$help_old" "$help_new")"
row "binary size" "$(awk -v b="$size_old" 'BEGIN { printf "%.2f MiB", b / 1048576 }')" \
    "$(awk -v b="$size_new" 'BEGIN { printf "%.2f MiB", b / 1048576 }')" "$(pct "$size_old" "$size_new")"

if [[ -n $budget ]]; then
    if awk -v a="$old_med" -v b="$new_med" -v p="$budget" 'BEGIN { exit !(b > a * (1 + p / 100)) }'; then
        err "median first frame regressed by more than $budget% ($(pct "$old_med" "$new_med"))"
        exit 1
    fi
    ok "median first frame within the $budget% budget"
fi
