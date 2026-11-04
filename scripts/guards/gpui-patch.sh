#!/usr/bin/env bash
# Guard: patches/gpui-0.2.2-startup.patch must be exactly the diff between
# pristine gpui 0.2.2 and crates/gpui (docs/gpui-fork.md).
#
# Also checks that every file the patch touches is described in
# crates/gpui/MDVIEW.md and carries a `mdview:` marker comment.
#
# The pristine source comes from the .crate archive (cargo's cache, or
# downloaded from static.crates.io into target/), verified against the
# crates.io checksum, so a locally edited ~/.cargo/registry/src can't fool it.
#
# --bless writes the patch without file timestamps (they change on every
# checkout and would make the patch churn); `patch -p1` doesn't need them. The
# comparison ignores timestamps either way.
#
# Usage: scripts/guards/gpui-patch.sh [--bless]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../lib.sh"
cd "$MDVIEW_ROOT"

VERSION=0.2.2
# From the crates.io index (https://index.crates.io/gp/ui/gpui, "cksum").
SHA256=979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707
PATCH=patches/gpui-$VERSION-startup.patch
CACHE=target/guards
PRISTINE=$CACHE/gpui-$VERSION

# Files the patch touches that are allowed to lack a `mdview:` marker. They
# only contain mechanical follow-ups of a marked change (destructuring the new
# WindowParams fields). Don't add to this list: mark new changes instead.
UNMARKED_OK=(src/window.rs src/platform/mac/window.rs)

fetch_pristine() {
    [[ -f $PRISTINE.verified && -d $PRISTINE ]] && return 0
    mkdir -p "$CACHE"
    local crate=""
    for c in "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/gpui-$VERSION.crate; do
        [[ -f $c ]] && crate=$c && break
    done
    if [[ -z $crate ]]; then
        crate=$CACHE/gpui-$VERSION.crate
        if [[ ! -f $crate ]]; then
            info "downloading gpui $VERSION from crates.io..."
            curl -fsSL --retry 3 -o "$crate.part" "https://static.crates.io/crates/gpui/gpui-$VERSION.crate" ||
                die "couldn't download gpui $VERSION (offline?). Run 'cargo fetch' or retry with network."
            mv "$crate.part" "$crate"
        fi
    fi
    local got
    got=$(sha256sum "$crate" | cut -d' ' -f1)
    if [[ $got != "$SHA256" ]]; then
        [[ $crate == "$CACHE"/* ]] && rm -f "$crate"
        die "checksum mismatch for $crate: got $got, expected $SHA256"
    fi
    rm -rf "$PRISTINE" "$PRISTINE.verified"
    tar -xzf "$crate" -C "$CACHE"
    # cargo writes this marker when it unpacks a crate into ~/.cargo/registry/src,
    # which is the baseline docs/gpui-fork.md diffs against (and crates/gpui
    # was copied from there, marker included).
    printf '{"v":1}' >"$PRISTINE/.cargo-ok"
    touch "$PRISTINE.verified"
}

generate_patch() {
    # Same flags as the command in docs/gpui-fork.md, run from a directory
    # where the two trees have the names the patch uses. diff exits 1 when the
    # trees differ, which is the normal case.
    (
        cd "$CACHE"
        ln -sfn "$MDVIEW_ROOT/crates/gpui" gpui-$VERSION-mdview
        diff -ru -x Cargo.lock -x .cargo_vcs_info.json -x MDVIEW.md \
            gpui-$VERSION gpui-$VERSION-mdview || [[ $? == 1 ]]
    ) | sed -E '/^(---|\+\+\+) /s/\t.*$//'
}

fetch_pristine
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
generate_patch >"$tmp"
[[ -s $tmp ]] || die "crates/gpui is identical to upstream gpui $VERSION? The generated patch is empty."

if [[ ${1:-} == --bless ]]; then
    if cmp -s "$PATCH" "$tmp"; then
        ok "$PATCH is up to date"
    else
        cp "$tmp" "$PATCH"
        ok "regenerated $PATCH (remember to update crates/gpui/MDVIEW.md)"
    fi
    exit 0
fi

# Compare without the ---/+++ timestamps, so a patch made with the command in
# docs/gpui-fork.md (which includes them) is accepted too.
committed=$(mktemp)
trap 'rm -f "$tmp" "$committed"' EXIT
sed -E '/^(---|\+\+\+) /s/\t.*$//' "$PATCH" >"$committed"

status=0
compare_snapshot "$committed" "$tmp" "crates/gpui (vs $PATCH)" "scripts/check.sh --bless" \
    "  crates/gpui differs from what $PATCH describes (docs/gpui-fork.md).
  Every change to crates/gpui needs a // mdview: comment, a regenerated patch
  and an updated crates/gpui/MDVIEW.md." || status=1

# Every touched file is explained in MDVIEW.md and marked in the code.
while read -r file; do
    rel=${file#src/}
    if ! grep -qF "$rel" crates/gpui/MDVIEW.md; then
        err "crates/gpui/$file is patched but not mentioned in crates/gpui/MDVIEW.md. Describe the change there."
        status=1
    fi
    if ! grep -q 'mdview:' "crates/gpui/$file"; then
        allowed=0
        for a in "${UNMARKED_OK[@]}"; do [[ $a == "$file" ]] && allowed=1; done
        if [[ $allowed == 0 ]]; then
            err "crates/gpui/$file is patched but has no '// mdview: <why>' comment (docs/gpui-fork.md)."
            status=1
        fi
    fi
done < <(sed -nE 's#^diff .* gpui-'"$VERSION"'-mdview/(\S+)$#\1#p; s#^Only in gpui-'"$VERSION"'-mdview/?(\S*): (\S+)$#\1/\2#p' "$tmp" | sed 's#^/##')

[[ $status == 0 ]] && ok "$PATCH matches crates/gpui ($(grep -c '^diff ' "$PATCH") files)"
exit $status
