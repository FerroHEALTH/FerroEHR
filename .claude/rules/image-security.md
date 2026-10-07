---
paths:
  - docker/**
  - security/vex/**
  - .trivyignore.yaml
  - trivy.yaml
  - scripts/security/**
  - .github/workflows/image-scan.yml
  - .github/workflows/containers.yml
  - .github/workflows/release.yml
  - .github/workflows/scan-and-tag.yml
  - .github/workflows/base-image-watcher.yml
---

# Published-image CVE remediation (the standing law)

The published container images are scanned continuously; this file is the
other half — what to DO with a finding, per finding class, so a fixable CVE
never sits waiting for someone to reinvent the remediation. Authored from
#2408, where 15 fixable HIGH findings sat in `ferroehr-postgres` because the
detection lane existed but the remediation path was tribal.

## The instrument layer (what fires when)

| Instrument | When | What it catches |
|---|---|---|
| the `scan-and-tag.yml` image-scan job (called by `containers.yml` and the release pipeline) | every image build: the app and viewer images by their pushed digests, both platforms, and the postgres image, before `apply-tags` tags any of them | what was known at build time; since #3675 the binaries in both images carry their Rust dependency list (`cargo auditable`), so the scan sees the Rust graph too |
| `image-scan.yml` | Mondays 07:13 UTC, on the PUBLISHED `:latest` refs | CVEs published after the release; files/updates ONE tracking issue — **the issue is the alert**, the run stays green (#2778) |
| `base-image-watcher.yml` | Mondays 08:13 UTC | a newer `postgres` patch tag on the pinned major, or the pinned tag re-pointed upstream (a same-version security respin); files/updates ONE tracking issue |
| `scripts/security/scan-images.sh` | on demand, locally | reruns the EXACT published-image scan (same `trivy.yaml`, `.trivyignore.yaml`, `security/vex/*.json`) against the published refs or a locally built candidate |
| the release pipeline's `vulnerability-record` job (`release.yml`) | every `v*` release, before `scan-and-tag` | EVERY finding (no severity floor, unfixed included, no ignore file) on each image digest and server binary; `scripts/checks/vex-coverage.sh` refuses the release while a finding has no OpenVEX judgement under `security/vex/`; the joined record is a release asset |

The first four read the same three config surfaces: `trivy.yaml` (severity floor +
`ignore-unfixed` + the ignore-file pointer), `.trivyignore.yaml` (per-CVE,
path-scoped adjudications), `security/vex/*.json` (the published arguments).
Never tune a lane by giving it its own flags — a lane that diverges from the
shared config silently changes what the gate means. The fifth is the one
deliberate exception and says so in its step: it is the record of every
finding rather than a gate on a subset, so it drops the floor, `ignore-unfixed`
and the ignore file, and judges each finding against `security/vex/` alone.

Platforms are explicit everywhere (#2412): the published index is dual-arch
(`linux/amd64` + `linux/arm64`) and trivy reads ONE variant per invocation
(`--platform`, default `linux/amd64` — the Trivy container-image guide), so
each lane scans both variants; a locally built `--candidate` is
single-platform by construction and says so.

## The remediation law (per finding class)

Attribute every finding to exactly one class BEFORE touching anything, from
the scan JSON (`Target`, `PkgName`, `FixedVersion`):

1. **Bytes we add** (the Rust binaries, init scripts): fix the dependency or
   code the normal way. Rust advisories go through `deny.toml` +
   `security/vex/rust-advisories.toml` per `security/vex/README.md`, never
   through `.trivyignore.yaml`.
2. **OS package in the postgres image, fix available in Debian
   (trixie/trixie-security)**: already self-healing — the Dockerfile's
   security-upgrade `RUN` layer pulls fixed packages at build time. Remedy =
   rebuild + re-release (a patch release; images republish only on tags).
   Verify first: `docker build -t ferroehr-postgres:candidate docker/postgres/` (under podman: `podman build --platform linux/amd64 …`, and again for arm64 — a digest-pinned base otherwise resolves to whichever platform the local store already holds)
   then `scripts/security/scan-images.sh --candidate ferroehr-postgres:candidate`
   must report 0.
3. **A newer upstream base exists** (the watcher's issue, or found while
   remediating class 2): base bump, full pin-site checklist below.
4. **Upstream-bundled binary whose fix lives in ITS toolchain** (a Go or
   other foreign-toolchain binary the base ships and we cannot rebuild): remove
   or replace it. The precedent is gosu (#3673): upstream's Go `gosu` carried a
   stream of Go standard-library advisories, so the Dockerfile deletes it and
   installs `docker/postgres/gosu`, a POSIX-sh wrapper over util-linux
   `setpriv` under the same name, and the image ships no Go code. A new bundled
   binary that cannot be removed goes to the owner before any VEX statement
   is written.
5. **OS package with no fix in Debian yet**: if the package is only used at
   build time (GnuPG, #3673), purge it in the Dockerfile; otherwise write one
   OpenVEX statement per id in `security/vex/postgres-os.openvex.json` (bump
   `version` + `timestamp`), argued from what the image runs and loads, with
   `affected` + an `action_statement` where a database session or the
   operator's configuration reaches the code. Never a `.trivyignore.yaml`
   entry.
6. **Never**: raise the severity floor, add a blanket suppression, scope an
   ignore wider than one CVE × one path, or VEX a finding we can fix by
   rebuilding ("A finding we can fix is fixed, not VEXed" —
   `security/vex/README.md`).

A fixable finding in the PUBLISHED images is milestoned into the CURRENT
patch milestone — the remedy is only real once a tag republishes the image.

## The base-bump pin-site checklist (postgres)

Machine-enforced agreement (`scripts/checks/image-labels.sh`, per-PR in CI):
the Dockerfile `FROM` digest ↔ its `base.name`/`base.digest` LABELs ↔ the
`containers.yml` labels ↔ every workflow's postgres service pins (the guard
scans `.github/workflows/*.yml`, so a new lane's pin is covered the day it
lands; it read `ci.yml` alone until #2775 and missed `sonar.yml`). Everything else
is a sweep — run `grep -rn '18\.<old-patch>'` over the tree and expect to
touch:

- `docker/postgres/Dockerfile` — `FROM` tag+digest, header comment, both
  `base.*` LABELs (the digest is the MANIFEST-LIST digest:
  `docker buildx imagetools inspect postgres:<tag>` → `Digest:`)
- `.github/workflows/containers.yml` — postgres lane comment + both label lines
- `.github/workflows/ci.yml` — its service-container pin + the job heading
- `.github/workflows/sonar.yml` — its own service-container pin
- `docs/VERSIONS.md` §Database, `docs/postgres-features.md` §Versioning note
  (latest-patch version + date, from postgresql.org — never copied forward),
  root `CLAUDE.md` §Tech stack, `docs/architecture.md` §PostgreSQL 18,
  the root `Cargo.toml` database comment, `.claude/rules/sqlx-conventions.md`
- `app/ferroehr/src/telemetry/provenance.rs` `PG_TARGET` + its
  `build_info.rs` test
- `tools/testkit/src/lib.rs` header comment
- `docker/postgres/README.md`, `website/book/src/installation/compose.md`
- `deploy/helm/ferroehr/README.md.gotmpl` (then `helm-docs`), the chart
  `version` bump if chart content changed this cycle, and
  `deploy/helm/validate.sh --update` for the goldens

Then re-check the VEX documents against a candidate scan with the release
lane's flags (no severity floor, unfixed included, no ignore file):
`security/vex/postgres-os.openvex.json` must cover every finding, a statement
whose CVE no longer fires is deleted (a stale `not_affected` is worse than no
VEX), and the upstream entrypoint must still call `gosu` the way the wrapper
expects (`gosu postgres "$BASH_SOURCE" "$@"`) and must still run with the
packages the GnuPG purge removes gone.

## Verification (what "fixed" means)

`scripts/security/scan-images.sh --candidate <image>` at 0 findings on the
rebuilt image, BEFORE merge. The Monday `image-scan.yml` run over the
re-released `:latest` is the closing evidence; the tracking issue closes via
the fixing PR and the next scheduled run confirms (it files a fresh issue if
anything still fires — a closed issue is never silently final).
