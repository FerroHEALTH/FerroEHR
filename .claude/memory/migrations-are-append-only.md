---
name: migrations-are-append-only
description: "Owner rulings 2026-09-09 + 2026-09-15 + 2026-11-20 (v6.0.0 baseline) — a migration that has SHIPPED (is at the latest vX.Y.Z tag or a vX.Y.Z-rcN tag above it) is never edited; a file on main but in no release is fixed in place; guarded by migration-immutability.sh against the release tag"
metadata:
  type: feedback
---

A migration file present at the latest release tag, or at a pre-release
tag of a version above it (#4425: rc2 shipped audit 0017, which #4383 then
edited), is **append-only**: never
edit, rename or delete it, including a comment or a typo; a schema change is a
NEW `sqlx migrate add` file and a wrong shipped migration is superseded. A file
that is on `main` but in NO release yet has been applied by no installation and
is **fixed in place** (owner 2026-09-15: the rewrite is breaking changes only;
no rename migrations, placeholder roles or compatibility paths).

**Why:** sqlx checksums every applied migration and refuses a database whose
checksum no longer matches, so an edit to a shipped file locks installations
out; an edit to an unshipped file locks nobody out and keeps the design clean.

**How to apply:** `scripts/checks/migration-immutability.sh --diff origin/main HEAD`
judges only files the highest `vX.Y.Z` tag and every pre-release tag above it
carry, reachable or not (all base files when the clone has no tag), and accepts
a restoration of a shipped file to its released bytes; a shipped SET is retired only whole and only with the boot
refusal naming its schema (`RETIRED_SETS`). The rule re-arms itself at
every release cut. Related: [[rewrite-breaks-everything-shipped-means-released]],
[[storage-rewrite-is-greenfield]].

**Owner ruling 2026-11-20 (#6139): v6.0.0 starts from a baseline.** The whole
v5 history (ext, clinical, party, audit, 188 files) was retired whole and
replaced by one baseline per set, numbered from `0001`, split by subject
(`000N_baseline_<subject>.sql`), creating each object once in its final shape:
no create-then-alter, no add-then-drop, no rename, no backfill, no re-grant.
This supersedes the append-only rule for the v5 sets only. A database carrying
v5 bookkeeping is refused at boot by `RETIRED_SETS` with the way forward (a new
database; the records moved through the archive dump and load). No upgrade
path and no compatibility layer: no product is in production. From the v6.0.0
tag the baseline is append-only again and every later change is a new file
after it. The templates under `migrations/templates/` render into the set
files and are not judged by the immutability guard themselves.
