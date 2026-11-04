#!/usr/bin/env bash
# Installs a git pre-commit hook that runs `scripts/check.sh --quick`.
#
#   scripts/install-hooks.sh              install (refuses to replace a foreign hook)
#   scripts/install-hooks.sh --force      install, backing up an existing hook
#   scripts/install-hooks.sh --uninstall  remove our hook
#
# Hooks live in the repository's common git dir, so one install covers every
# worktree. The hook runs the check.sh of the worktree being committed, and
# does nothing in a worktree whose branch doesn't have it yet.
# Skip it once with `git commit --no-verify`.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$MDVIEW_ROOT"

MARKER="# mdview pre-commit hook (scripts/install-hooks.sh)"
hooks=$(git rev-parse --path-format=absolute --git-path hooks)
hook=$hooks/pre-commit

case ${1:-} in
--uninstall)
    if [[ -f $hook ]] && grep -qF "$MARKER" "$hook"; then
        rm "$hook"
        ok "removed $hook"
    else
        note "no mdview hook installed at $hook"
    fi
    exit 0
    ;;
--force | "") ;;
*) die "unknown option: $1" ;;
esac

if git config core.hooksPath >/dev/null; then
    note "core.hooksPath is set to '$(git config core.hooksPath)'; installing there."
fi
mkdir -p "$hooks"

if [[ -e $hook ]] && ! grep -qF "$MARKER" "$hook"; then
    if [[ ${1:-} != --force ]]; then
        die "$hook already exists and isn't ours. Re-run with --force to back it up and replace it."
    fi
    mv "$hook" "$hook.bak.$(date +%s)"
    note "backed up the existing hook"
fi

cat >"$hook" <<EOF
#!/usr/bin/env bash
$MARKER
# Runs the quick checks of the worktree being committed. Skip once with
# git commit --no-verify. Note: it checks the working tree, not just what's staged.
top=\$(git rev-parse --show-toplevel) || exit 1
if [[ ! -x \$top/scripts/check.sh ]]; then
    exit 0 # a branch from before check.sh existed
fi
exec "\$top/scripts/check.sh" --quick
EOF
chmod +x "$hook"
ok "installed $hook (runs scripts/check.sh --quick before each commit)"
