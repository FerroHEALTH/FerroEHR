---
name: quickstart-from-release-download
description: every product ships one self-contained compose.yaml as a release asset; the quick start downloads it with curl, never clones and never unpacks a bundle (owner, 2026-11-10)
metadata:
  type: feedback
---

Every product has one self-contained `compose.yaml`: its configs and init
scripts inline under Compose `configs:` with `content:`, its images pinned by
digest. The release attaches that file as an asset. The quick start (README
and book) is `curl -fsSLO
https://github.com/FerroHEALTH/<Repo>/releases/latest/download/compose.yaml`,
a few `openssl` lines for the secret files, then `docker compose up --wait`.
No `git clone`, no tarball bundle.

**Why:** owner, 2026-11-10: one compose.yaml is "the most simple and best
direction and fastest for everybody", and a clone runs main's configuration
against an older image.

**How to apply:** the CI tour runs the repository's compose.yaml exactly as
shipped. Licensed data (terminology) stays an optional user-supplied mount.
