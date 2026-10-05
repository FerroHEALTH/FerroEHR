#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compose hardening guard (OWASP Docker Security Cheat Sheet; issues #1993–#2001).
#
# The Kubernetes path is hardened by `deploy/helm/ferroehr/values.yaml`, which a
# golden-render test pins. The Compose path had no equivalent, so this is it:
# every committed compose artifact must keep the isolation posture, and the
# quickstart must not publish a port on every interface.
#
# What each rule prevents:
#
#   1. `cap_drop: [ALL]` on every service — the default set grants ~14
#      capabilities, most of which no service here uses. A container that never
#      needs CAP_NET_RAW should not be able to forge packets if it is compromised.
#   2. `no-new-privileges:true` — blocks a setuid binary inside the container from
#      raising privileges, the Kubernetes `allowPrivilegeEscalation: false`
#      equivalent.
#   3. No `privileged: true`, ever — it hands over the host.
#   4. No `seccomp=unconfined` / `apparmor=unconfined` — Docker applies its
#      default seccomp profile unless overridden, and overriding it removes a
#      block on ~44 syscalls (docs.docker.com/engine/security/seccomp). The
#      posture is kept by the ABSENCE of the override, so absence is what is
#      checked.
#   5. No `/var/run/docker.sock` mount — the daemon socket is root on the host
#      (docs.docker.com/engine/security: "only trusted users should be allowed to
#      control your Docker daemon").
#   6. Published ports bind an explicit host address — a port published on
#      0.0.0.0 is DNAT'd ahead of the host firewall's chains, so `ufw deny` does
#      not stop it (docs.docker.com/engine/network/packet-filtering-firewalls).
#
# Usage: scripts/checks/compose-hardening.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

# Every committed compose artifact: the root files plus the `services:`-bearing
# YAML under docker/.
mapfile_compat() {
  { find . -maxdepth 1 -name 'docker-compose*.yml' -print
    find docker -name '*.yml' -print 2>/dev/null; } | sort -u
}

failures=0
report() {
  printf 'compose-hardening: %s\n' "$1" >&2
  failures=$((failures + 1))
}

for f in $(mapfile_compat); do
  [[ -f "$f" ]] || continue
  grep -q '^services:' "$f" || continue

  # Comments are excluded from every content rule below: this guard's own rules
  # are DOCUMENTED in the compose headers, so scanning comments would report the
  # explanation as a violation.
  body=$(grep -vE '^\s*#' "$f")

  # (3) never privileged
  if printf '%s\n' "$body" | grep -qE '^\s*privileged:\s*true'; then
    report "$f publishes a privileged service — it hands the host to the container"
  fi
  # (4) the default seccomp/apparmor profiles stay
  if printf '%s\n' "$body" | grep -qE 'seccomp[:=]unconfined|apparmor[:=]unconfined'; then
    report "$f disables seccomp or AppArmor — the default profile blocks ~44 syscalls"
  fi
  # (5) the daemon socket is never handed over
  if printf '%s\n' "$body" | grep -q '/var/run/docker.sock'; then
    report "$f mounts the Docker daemon socket — that is root on the host"
  fi
  # (6) a published port names its host interface
  while IFS=: read -r line _; do
    [[ -n "${line:-}" ]] || continue
    report "$f:$line publishes a port without a host address — it binds every \
interface and bypasses the host firewall"
  done < <(grep -nE '^\s+- "?[0-9$][^:]*:[0-9]+"?\s*$' "$f" | grep -v 'BIND_HOST' || true)

  # (1)+(2) every service DEFINITION declares the isolation floor.
  #
  # Only a block that declares `image:` or `build:` is a definition; a block
  # carrying just `environment:` or `configs:` is an OVERLAY that extends a
  # service defined elsewhere, and Compose merges the two — so the base file's
  # `cap_drop` already applies and repeating it in the overlay would be
  # duplication that can drift
  # (docs.docker.com/compose/how-tos/multiple-compose-files/merge).
  #
  # A service named in BOTH the base file and an overlay is likewise exempt in the
  # overlay, even when the overlay restates `image:` (which
  # `docker-compose.dev.yml` does to swap in the from-source tags): Compose
  # merges the two definitions, and restating `security_opt` there is not merely
  # redundant — it makes `docker compose config` fail with "items at 0 and 1 are
  # equal", because list-valued keys concatenate.
  base_services=""
  if [[ "$f" != "./docker-compose.yml" ]] && [[ -f docker-compose.yml ]]; then
    base_services=$(awk '
      /^services:/ { in_services = 1; next }
      /^[a-z]/     { in_services = 0 }
      in_services && /^  [a-z0-9_-]+:[[:space:]]*$/ { svc = $1; sub(/:$/, "", svc); printf "%s ", svc }
    ' docker-compose.yml)
  fi
  missing=$(awk -v base_list="$base_services" '
    BEGIN { n = split(base_list, a, " "); for (i = 1; i <= n; i++) if (a[i] != "") base[a[i]] = 1 }
    /^services:/ { in_services = 1; next }
    /^[a-z]/     { if (in_services) { check(); in_services = 0 } }
    in_services && /^  [a-z0-9_-]+:[[:space:]]*$/ {
      check()
      svc = $1; sub(/:$/, "", svc)
      defines = 0; caps = 0; noesc = 0
      next
    }
    in_services && /^ +(image|build):/  { defines = 1 }
    in_services && /^ +cap_drop:/       { caps = 1 }
    in_services && /no-new-privileges:true/ { noesc = 1 }
    END { check() }
    function check() {
      if (svc != "" && defines && !(svc in base) && (!caps || !noesc)) {
        printf "%s%s", (out ? ", " : ""), svc
        out = 1
      }
      svc = ""
    }
  ' "$f")
  if [[ -n "$missing" ]]; then
    report "$f defines service(s) without the isolation floor (cap_drop + \
no-new-privileges): $missing"
  fi
done

if [[ "$failures" -gt 0 ]]; then
  echo "compose-hardening: $failures violation(s) — see above." >&2
  exit 1
fi
echo "compose-hardening: OK."
