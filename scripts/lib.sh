# Shared helpers for scripts/*.sh. Source it; don't run it.
# shellcheck shell=bash

# Repo root, whatever the caller's cwd is.
MDVIEW_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
export MDVIEW_ROOT

if [[ -t 1 && -z ${NO_COLOR:-} ]]; then
    C_RED=$'\e[31m' C_GREEN=$'\e[32m' C_YELLOW=$'\e[33m' C_BOLD=$'\e[1m' C_DIM=$'\e[2m' C_OFF=$'\e[0m'
else
    C_RED='' C_GREEN='' C_YELLOW='' C_BOLD='' C_DIM='' C_OFF=''
fi

info() { printf '%s\n' "$*"; }
note() { printf '%snote:%s %s\n' "$C_YELLOW" "$C_OFF" "$*"; }
ok() { printf '%sok:%s %s\n' "$C_GREEN" "$C_OFF" "$*"; }
err() { printf '%serror:%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; }
die() {
    err "$*"
    exit 1
}

# Compare a freshly generated file with a committed snapshot. On a mismatch,
# print the diff and the bless hint, and return 1.
#   compare_snapshot <committed> <generated> <what> <bless-command> [why]
compare_snapshot() {
    local committed=$1 generated=$2 what=$3 bless=$4 why=${5:-}
    if [[ ! -f $committed ]]; then
        err "$what: the snapshot is missing."
        info "  Create it with: $bless"
        return 1
    fi
    if diff -q "$committed" "$generated" >/dev/null; then
        return 0
    fi
    err "$what changed:"
    diff -u --label "committed" --label "now" "$committed" "$generated" | sed 's/^/    /' | head -n 80
    [[ -n $why ]] && info "$why"
    info "  If the change is intentional, bless it: ${C_BOLD}$bless${C_OFF}"
    return 1
}
