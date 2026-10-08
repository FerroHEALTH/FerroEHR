---
name: podman-not-docker
description: "The owner's machine runs podman, not Docker — how to point testkit and container tooling at it"
metadata:
  node_type: memory
  type: user
  originSessionId: 44d54adc-7226-4d71-bc1a-2ecc4709cb40
  modified: 2026-10-07T19:07:53.366Z
---

The owner uses **podman** locally, never Docker (stated 2026-10-07). There is no `docker` binary; `docker-compose` exists at /usr/local/bin.

**How to apply:**
- Test suites do NOT run locally (see [[tests-run-in-pr-ci]]: the machine lacks the RAM); they run in the PR's CI. Where a podman-backed tool needs a Docker socket (testkit, the deploy probes, docker-compose), point it at podman: `DOCKER_HOST=unix://$(podman machine inspect --format '{{.ConnectionInfo.PodmanSocket.Path}}') TESTCONTAINERS_RYUK_DISABLED=true`. A `docker` shim that maps `docker compose` to docker-compose and everything else to podman lets scripts that call `docker` run.
- If `podman compose version` says "connection refused" while `podman machine list` shows the VM up, `podman machine stop && podman machine start` fixes it.
- Scripts calling `docker compose` (e.g. `scripts/ui-e2e.sh`, the screenshot refresh) do not run as-is here; ask before shimming them.
