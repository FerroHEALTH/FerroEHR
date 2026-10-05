#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The default Grafana dashboard has ONE canonical home — the Helm chart file —
# and one embedded copy: the compose observability overlay must stay standalone
# (downloadable with no repo checkout), so it inlines the same JSON with `$`
# escaped as `$$` against compose interpolation. This guard holds the two
# byte-identical (modulo that escaping); issue #2641.
#
# Usage: scripts/checks/dashboard-drift.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

CANONICAL=deploy/helm/ferroehr/files/dashboards/ferroehr-overview.json
OVERLAY=docker-compose.observability.yml

[[ -f "$CANONICAL" ]] || { echo "dashboard-drift: missing $CANONICAL" >&2; exit 1; }

inline=$(awk '
  found && !/^      / && NF { exit }
  found { sub(/^      /, ""); print }
  /^    content: \|$/ { found = 1 }
' "$OVERLAY" | sed 's/\$\$/$/g')

[[ -n "$inline" ]] || { echo "dashboard-drift: no inline dashboard found under 'content: |' in $OVERLAY" >&2; exit 1; }

if ! diff <(printf '%s\n' "$inline") "$CANONICAL" >/dev/null; then
  echo "dashboard-drift: the overlay's inline dashboard differs from $CANONICAL" >&2
  echo "regenerate the inline copy from the canonical file (escape \$ as \$\$)" >&2
  diff <(printf '%s\n' "$inline") "$CANONICAL" | head -20 >&2
  exit 1
fi
echo "dashboard-drift: OK."
