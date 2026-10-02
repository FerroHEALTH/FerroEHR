# The hosted sandbox (sandbox.ferroehr.eu)

sandbox.ferroehr.eu is the public demo: the viewer as the landing
surface over a live CDR, the demo credentials `ferroehr` / `ferroehr`, demo
data wiped nightly. This directory is the whole deployment posture, mirrored
from Veredictum's `deploy/hosted` (#2974).

## Why a box, and not a platform

The sandbox used to run on Vercel's container platform, and the platform kept
fighting the server: instances scale in and out under a request model, boots
re-run the migrations on visitor traffic, and the nightly reset had to promote
whole deployments to get one fresh boot (#2941, #2947, #2948 are the
workaround trail — and it was slow). So the sandbox runs on one machine that
does not stop. The cost of that choice is ownership: the firewall, the
certificate, the deploy path and the answer to "is it up" are all things this
directory states.

## The machines

Two Hetzner **CX33** servers — 4 shared vCPU, 8 GB RAM, 80 GB NVMe SSD, 20 TB
included traffic each; €8.49/month net apiece (€0.0136/h, DE list price since
15 June 2026,
<https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/>) —
in Nuremberg (`eu-central`), joined by a private network
(`ferroehr-sandbox-net`, 10.0.0.0/24) and a spread placement group (different
physical hosts, so one host failure never takes both):

- **The app box** (`ferroehr`, 167.233.172.220 / 2a01:4f8:1c16:5d4::/64,
  private 10.0.0.3): the CDR, the viewer and Caddy, and FerroTERM (the
  terminology server, on the compose network only: no port, no Caddy route,
  the CDR its only caller, #3304) — provisioned once from
  `cloud-init.yaml`, resized in place from the original CPX22 on 2026-09-01.
  The compose memory limits are sized for this box; on a resize they move
  with it, and neither is a code change.
- **The database box** (`ferroehr-sandbox-postgres`, 2.28.64.110 /
  2a01:4f8:1c16:c964::/64, private 10.0.0.2): the published
  `ferroehr-postgres` image (PostgreSQL 18), provisioned once from
  `cloud-init-postgres.yaml` on 2026-09-01 (#3015). PostgreSQL binds to the
  private address alone; the public side carries key-only SSH and nothing
  else. `postgres.ferroehr.eu` names the PUBLIC address for SSH and
  operations — its 5432 never answers, by design.

The connection string lives in the app box's `.env` and nowhere else; CI
never holds it. The release deploy lane touches the APP box only — the
database box has no deploy lane, and its image moves by hand
(`docker compose pull && docker compose up -d` in `/opt/ferroehr-postgres`).

Two operational lessons, learned live at the cutover and load-bearing:

- **A `DATABASE_URL` change needs `docker compose up -d` (a recreate), never
  `docker compose restart`** — restart reuses the container's original
  environment, so the CDR keeps talking to the old database while readiness
  honestly reports UP about the wrong server.
- **The app box's `.env` is fed into the CDR container wholesale**, and the
  CDR's strict boot sweep refuses any `FERROEHR_*` name its release does not
  know — an `.env` line staged for a FUTURE release keeps the CURRENT one
  from booting. Stage such lines commented out.

## What is in this directory

| File | What it is | How the box gets it |
|---|---|---|
| `cloud-init.yaml` | A fresh box to serving state on first boot: the `deploy` user, key-only SSH, both firewalls, unattended upgrades, Docker, capped logs, and the one command the CI key may run | the server's user data at creation, once |
| `docker-compose.yml` | What runs on the box: the CDR and the viewer with their healthchecks and memory limits, behind Caddy | baked into the ferroehr image at `/opt/sandbox-posture/`, and `deploy.sh` installs it from the image it pulled |
| `Caddyfile` | Automatic TLS, and the routing table (the CDR owns `/ferroehr/*`, `/health*`, `/management*`, `/.well-known/*`; the viewer is everything else) | the same way; a change to it restarts Caddy |
| `ferroehr.sandbox.toml` | The CDR's sandbox posture (demo user, admin API on per #2965, management off) | the same way |
| `ferroehr-viewer.sandbox.toml` | The viewer's sandbox posture | the same way |
| `env.example` | A copy-to-`.env` template: the DB box's private-network DSN and the image references | never. `.env` is the operator's file, written by hand on the box |
| `cloud-init-postgres.yaml` | The DATABASE box, fresh to serving state: the `deploy` user, key-only SSH, both firewalls, Docker, and the `ferroehr-postgres` compose posture bound to the private address | the DB server's user data at creation, once |
| `ferroterm-index.env` | The record of the terminology index the box carries: its artifact layout, the `ferroterm-build` version and the editions. `scripts/checks/ferroterm-index-layout.sh` refuses a FerroTERM pin in `docker-compose.yml` whose layout differs | never. The index is built off-box and copied by the operator (below), and this file changes in the same pull request as the pin |
| `docker/terminology/seed/` (in the repository) | The licence-free shaped code systems FerroTERM serves on the box | baked into the ferroehr image at `/opt/sandbox-posture/terminology/`; the compose file mounts it into FerroTERM straight from the image, so `deploy.sh` never handles it |

The box holds **no checkout of this repository** and fetches nothing from it
over the network: the posture travels inside the published ferroehr image, so
a posture change arrives at a release, with that artifact's provenance. The
`hosted-deploy-script` CI job extracts `deploy.sh` out of `cloud-init.yaml`
and shellchecks it, and checks that `docker/Dockerfile` bakes every posture
path the script reads.

## How a deploy happens

`.github/actions/hosted-deploy` is the only thing that deploys, and it carries
every step. Three callers:

1. **A real release** — the `sandbox` job in `release.yml`, after scan-and-tag
   moves `:latest`. A prerelease moves no `:latest`, so it deploys nothing.
   The job wipes the schemas FIRST (verb `wipe`), then deploys: the recreate
   is the fresh boot that migrates the empty database, so a release that
   changes the storage generation lands too (the v4.3.1 image refused the
   first-generation database it found, #3467). The `sandbox-reseed` leg then
   seeds only.
2. **A manual `workflow_dispatch`** of `.github/workflows/hosted-deploy.yml`.
   It wipes nothing: a redeploy of the same image keeps the day's demo data.
3. **The reseed** (`sandbox-reseed.yml`, nightly; the release calls it
   seed-only) — verbs `wipe` (drop the CDR's five schemas on the private
   database box, plus the four superseded first-generation ones a box seeded
   before the storage rewrite still carries, with the DSN this box already
   holds) and `restart` (recreate the CDR so a fresh boot re-runs the
   migrations against the wiped database).

The deploy key is restricted on the box to `deploy.sh` (`command=` in
`cloud-init.yaml`), which reads `SSH_ORIGINAL_COMMAND` and accepts exactly
`deploy`, `restart` and `wipe`. The lane builds nothing; it asserts by digest that
`:latest` is the tagged image, refuses an image that does not declare
`eu.ferroehr.image.carries-sandbox-posture=true`, deploys through
[`rubentalstra/hetzner-deploy-action`](https://github.com/rubentalstra/hetzner-deploy-action),
and then fetches `https://sandbox.ferroehr.eu/ferroehr/rest/status` FROM the
runner, requiring the served `server_version` — a bare 200 is not proof, the
deployment being replaced answers 200 too.

`HOSTED_SSH_KEY` and `HOSTED_KNOWN_HOSTS` are ENVIRONMENT secrets in the
`hosted` environment, so nothing else in this repository can reach the key.
No Hetzner API token exists in CI at all — the deploy talks to the host and
nothing else, so a leaked key cannot destroy the server.

## The demo dataset

`scripts/sandbox/reseed.sh` fills the wiped sandbox, driving only the public
REST API: the seed is the same surface a visitor uses, so a green seed run is
itself evidence that the deployment serves what it claims. The script is a
walker over `scripts/sandbox/seed/manifest.json`, and the bodies live as files
beside it, so **adding content is editing data**: a template slug, an EHR
subject, an AQL file, a party. The walker itself never changes.

One run loads, in about 590 requests:

| What | How much |
|---|---|
| ADL 1.4 operational templates | 16 (the curated CKM pack, `corpus/templates/ckm/`) |
| ADL 2 archetypes | 235 — 232 from the vendored 2013 CKM corpus plus 3 of our own |
| ADL 2 templates | 5 (4 of our own plus one carried by the corpus); two of ours are SOURCE templates the CDR flattens against that archetype library |
| EHRs | 8, each with a distinct subject and a mixed record |
| Compositions | 182 (166 ADL 1.4, 16 from CDR-generated ADL 2 examples) |
| Further versions | 6 COMPOSITION, 2 EHR_STATUS |
| Demographic parties | 6 persons, 3 roles, 2 party relationships |
| Directories | one FOLDER tree per EHR, its items real composition references |
| Stored AQL queries | 5, under the `eu.ferroehr.sandbox` namespace |

The ADL 2 archetype step offers the whole 322-file corpus, parents before
children, and pins both outcomes in the manifest: 233 artefacts stored, 89
refused with a `422`. The refusals are AOM2 validity findings on the 2013
conversions: constraints on attributes the RM does not declare
(`DV_QUANTITY.property`, and the RM functions `is_integral` and `offset` the
machine-readable RM omits), languages, slots and node ids that do not conform
to the flat parent, and eight children of refused parents that therefore have
no flat parent to validate against. Every refused file is adjudicated by name
in the corpus gate (`crates/openehr-adl/tests/it/ckm_archetype_packs.rs`), and
a change in either count fails the run instead of passing quietly. Two of the eight EHRs end the run with
a second `EHR_STATUS` version: one not queryable, one not modifiable, so both
flags are visible on live data.

Measured on 2026-09-02 against the composed quickstart stack on an 8-CPU/8 GB
Docker VM: **39–45 s** from an empty schema, under a second on a repeat run.
The data is ~9 MB of canonical composition JSON and leaves a **24 MB**
database. Against the box, the ~590 sequential HTTPS round trips dominate the
wall clock, and the `seed` job's `timeout-minutes: 20` covers that with room
for the 12 × 15 s retry budget.

Repeats are safe. The definition surfaces answer `409` when the artefact is
already there and the walker accepts that; the EHRs resolve by subject before
being created; and a marker EHR (`sandbox-seed-complete`) written last records
completion, so a second run reports "already seeded" and touches nothing.

The terminology seed under `docker/terminology/seed/` belongs to the
conformance lane and stays out of this deployment. Its two synthetic code
systems live under `cnf.example.test`, and no example in the CKM pack
references them; the coded text those examples do carry (`openehr`, ISO 639-1,
ISO 3166-1, plus external SNOMED CT references) resolves against the server's
own in-process terminology bundle.

## Bootstrap and recovery

The first posture on a fresh box (or recovery from a broken one) is manual by
design: copy this directory's four posture files plus a filled-in `.env` to
`/opt/ferroehr-sandbox/` and run `docker compose up -d` as the `deploy` user.
From the next release on, `deploy.sh` keeps them current from the image, and
each replaced file leaves a `.prev` copy beside it for rollback.
`deploy.sh` itself does not self-update: a new version reaches the box by
hand, or by rebuilding the box from `cloud-init.yaml`.

## Rebuilding the terminology index

FerroTERM refuses an index built for another artifact layout, so a pin bump
that moves the layout needs a rebuilt index on the box before the release's
`sandbox` leg runs; the `ferroterm-index-layout` CI job fails the pull request
until `ferroterm-index.env` says so. Build both indexes off-box with the
`ferroterm-build` that the pinned image ships, from the licensed releases:

```sh
IMG=ghcr.io/ferrohealth/ferroterm:<version>@sha256:<digest>
docker run --rm --user "$(id -u):$(id -g)" -v <snomed-dir>:/in:ro -v <out>:/out \
  --entrypoint /usr/local/bin/ferroterm-build "$IMG" \
  --rf2 /in/SnomedCT_InternationalRF2_PRODUCTION_<date>.zip --out /out/int
docker run --rm --user "$(id -u):$(id -g)" -v <loinc-dir>:/in:ro -v <out>:/out \
  --entrypoint /usr/local/bin/ferroterm-build "$IMG" \
  --loinc /in/Loinc_<version>.zip --out /out/loinc
```

Copy `<out>` to `/opt/ferroehr-sandbox/ferroterm-index.new/`, check that both
`manifest.json` files carry the expected `storeLayout`, swap the directory for
`ferroterm-index` (keeping the old one until the new one serves), set
`FERROTERM_IMAGE` in `.env` when the box must run ahead of the image's
posture, and run `docker compose up -d ferroterm`. The release archives never
reach the box.

## How it is watched

Four layers, each answering a different question: the containers' own
healthchecks plus `restart: unless-stopped`; the deploy's own verification;
`.github/workflows/hosted-watch.yml` every fifteen minutes (up, the served
version against the release pointer, and the viewer's landing markup — one
reused issue, closed on recovery); and size-capped log rotation, because a
box that fills its disk fails in a way that looks like nothing at all.

## What the box keeps, and what it never holds

Nothing durable and nothing secret beyond the one `.env`: the containers are
rebuilt from published images, the data is wiped nightly by design
(`sandbox-reseed.yml` — the wipe runs ON this box through the deploy
script's `wipe` verb, over the private network), and no signing key, token
or credential beyond the database DSN exists on the host. **No backups**, deliberately.
