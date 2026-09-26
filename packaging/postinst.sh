#!/bin/sh
# postinst for rism (deb: "$1"=configure, rpm: "$1"=1 install / 2 upgrade,
# archlinux: no args). Warn-only by design: a running MCP server keeps its
# old binary mapped — killing it mid-session would be worse than the stale
# process. Never fails: a non-zero exit aborts the package manager's
# transaction on deb/rpm.
set -u

if command -v pgrep >/dev/null 2>&1 && pgrep -x rism >/dev/null 2>&1; then
    echo "warning: rism is currently running; restart it (or its MCP host)" \
        "to use the new binary." >&2
fi
exit 0
