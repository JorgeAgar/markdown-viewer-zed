#!/usr/bin/env bash
# Guard: size of the release binary.
#
# The warm path (`mdview FILE` while a window is open) pays for loading the
# executable: relocations and page faults grow with its size (docs/startup.md).
# tests/guards/binary-size.txt holds the blessed size in bytes; this fails if
# target/release/mdview is more than 3% larger, and suggests a re-bless if it's
# more than 3% smaller.
#
# Release builds take minutes, so this only runs on a binary that is newer
# than every source file (otherwise it would compare a stale build) unless
# --build is given, which runs `cargo build --release` first.
#
# Usage: scripts/guards/binary-size.sh [--build] [--bless]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../lib.sh"
cd "$MDVIEW_ROOT"

SNAPSHOT=tests/guards/binary-size.txt
BIN=target/release/mdview
TOLERANCE_PCT=3

build=0 bless=0
for a in "$@"; do
    case $a in
    --build) build=1 ;;
    --bless) bless=1 ;;
    *) die "unknown argument: $a" ;;
    esac
done

if [[ $build == 1 ]]; then
    info "building the release binary (full LTO, takes a minute or more)..."
    cargo build --release --locked --quiet
fi

if [[ ! -x $BIN ]]; then
    note "binary size: skipped, $BIN doesn't exist (use scripts/check.sh --release)"
    exit 0
fi
newer=$(find Cargo.toml Cargo.lock src crates -newer "$BIN" -type f \
    -not -path '*/target/*' -print -quit 2>/dev/null || true)
if [[ -n $newer ]]; then
    note "binary size: skipped, $BIN is older than $newer (use scripts/check.sh --release)"
    exit 0
fi

size=$(stat -c %s "$BIN")
if [[ $bless == 1 ]]; then
    {
        echo "# Size in bytes of target/release/mdview (stripped, fat LTO, x86_64 Linux)."
        echo "# Checked by scripts/guards/binary-size.sh (fails if it grows by more than $TOLERANCE_PCT%)."
        echo "# Bless after an intentional change: scripts/check.sh --release --bless"
        echo "$size"
    } >"$SNAPSHOT"
    ok "blessed $SNAPSHOT: $size bytes"
    exit 0
fi

[[ -f $SNAPSHOT ]] || die "$SNAPSHOT is missing. Create it with: scripts/check.sh --release --bless"
want=$(grep -v '^#' "$SNAPSHOT" | head -n1 | tr -d '[:space:]')
[[ $want =~ ^[0-9]+$ ]] || die "$SNAPSHOT doesn't contain a size in bytes"

mib() { awk -v b="$1" 'BEGIN { printf "%.2f MiB", b / 1048576 }'; }
delta_pct=$(awk -v a="$size" -v b="$want" 'BEGIN { printf "%+.2f", (a - b) * 100 / b }')
summary="$BIN is $(mib "$size") ($size bytes), $delta_pct% vs the blessed $(mib "$want")"

if ((size * 100 > want * (100 + TOLERANCE_PCT))); then
    err "$summary (limit +$TOLERANCE_PCT%)."
    info "  A bigger binary slows the warm path (docs/startup.md). Find out what grew"
    info "  (new dependency? new feature?) and measure it: scripts/startup-bench.sh --ref main"
    info "  If it's intentional and measured, bless it: ${C_BOLD}scripts/check.sh --release --bless${C_OFF}"
    exit 1
fi
if ((size * 100 < want * (100 - TOLERANCE_PCT))); then
    note "$summary. Nice! Bless the new size so later growth is caught: scripts/check.sh --release --bless"
    exit 0
fi
ok "$summary"
