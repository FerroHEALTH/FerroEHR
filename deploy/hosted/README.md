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
  private 10.0.0.3): the CDR, the viewer and Caddy, FerroTERM (the
  terminology server, on the compose network only: no port, no Caddy route,
  the CDR its only caller, #3304), and FerroPIX (the IHE patient identity
  source the CDR resolves patients through, on loopback only, #4242) with the
  development token issuer both of them trust — provisioned once from
  `cloud-init.yaml`, resized in place from the original CPX22 on 2026-09-01.
  The compose memory limits are sized for this box (the budget below); on a
  resize they move with it, and neither is a code change.
- **The database box** (`ferroehr-sandbox-postgres`, 2.28.64.110 /
  2a01:4f8:1c16:c964::/64, private 10.0.0.2): the published
  `ferroehr-postgres` image (PostgreSQL 18), provisioned once from
  `cloud-init-postgres.yaml` on 2026-09-01 (#3015). It holds two databases:
  `ferroehr` for the CDR and `ferropix` for FerroPIX, each with its own
  role. PostgreSQL binds to the
  private address alone; the public side carries key-only SSH and nothing
  else. `postgres.ferroehr.eu` names the PUBLIC address for SSH and
  operations — its 5432 never answers, by design.

The connection strings (`DATABASE_URL` for the CDR, `FERROPIX_DATABASE_URL`
for FerroPIX) and FerroPIX's sealing key (`FERROPIX_DATA_KEY`) live in the
app box's `.env` and nowhere else; CI never holds them. The release deploy
lane touches the APP box only — the
database box has no deploy lane, and its image moves by hand. It runs
`ghcr.io/ferrohealth/ferroehr-postgres:5.5.0@sha256:dfe1d5c1bb95d31d3f6ff00fec03cd76ae20d57d20e6225b31e5bc68adf7fa81`,
the release tag and index digest `cloud-init-postgres.yaml` pins as the
`FERROEHR_POSTGRES_IMAGE` default (`scripts/checks/compose-image-tags.sh`
holds the two to the same reference). The pin moves after a release
publishes, since the digest exists only then, and step 6 of the rollout
script below moves the box onto it.

Two operational lessons, learned live at the cutover and load-bearing:

- **A `DATABASE_URL` change needs `docker compose up -d` (a recreate), never
  `docker compose restart`** — restart reuses the container's original
  environment, so the CDR keeps talking to the old database while readiness
  honestly reports UP about the wrong server.
- **The app box's `.env` was fed into the CDR container wholesale**, and the
  CDR's strict boot sweep refuses any `FERROEHR_*` name its release does not
  know — an `.env` line staged for a FUTURE release kept the CURRENT one
  from booting. The compose file now hands the CDR `DATABASE_URL` alone; the
  rest of `.env` is compose substitution, read by no container's sweep. A
  line a container of the running posture does receive is still staged
  commented out until the release that knows it.

## The memory budget (app box)

A CX33 has 8 GB, of which the kernel reports about 7.6 GiB. The compose
limits, with what each used on 2026-10-27 in the local rehearsal of #4242
(idle after a full seed, without the SNOMED CT and LOINC indexes):

| Service | Limit | Measured |
|---|---|---|
| CDR | 3072m | 34 to 213 MB |
| FerroTERM | 1536m | 26 MB with the seed alone; the SNOMED CT International index adds about 702 MB resident (#3304) |
| viewer | 1024m (was 1536m) | 6 MB |
| FerroPIX | 512m (FerroPIX's chart default) | 4 MB |
| development issuer (JVM) | 256m | 136 to 144 MB |
| Caddy | 128m | 13 MB |
| **Total** | **6528m** | |

That leaves about 1.1 GiB for the OS, Docker and the page cache. The one-shots
(`identity-issuer-ready`, `ferropix-seed`) live for seconds and carry no limit.

## What is in this directory

| File | What it is | How the box gets it |
|---|---|---|
| `cloud-init.yaml` | A fresh box to serving state on first boot: the `deploy` user, key-only SSH, both firewalls, unattended upgrades, Docker, capped logs, and the one command the CI key may run | the server's user data at creation, once |
| `docker-compose.yml` | What runs on the box: the CDR and the viewer with their healthchecks and memory limits, behind Caddy | baked into the ferroehr image at `/opt/sandbox-posture/`, and `deploy.sh` installs it from the image it pulled |
| `Caddyfile` | Automatic TLS, and the routing table (the CDR owns `/ferroehr/*`, `/health*`, `/management*`, `/.well-known/*`; the viewer is everything else) | the same way; a change to it restarts Caddy |
| `ferroehr.sandbox.toml` | The CDR's sandbox posture (demo user, admin API on per #2965, management off) | the same way |
| `ferroehr-viewer.sandbox.toml` | The viewer's sandbox posture | the same way |
| `ferropix.sandbox.toml` | FerroPIX's sandbox posture: the loopback base, the development issuer, the synthetic identifier domain `urn:oid:2.999.1.1` fed by `sandbox-source`, and the pseudonymisation domain `urn:oid:2.999.7.1` whose consumer is the CDR (`ferroehr`) | the same way |
| `ferropix-seed.sh` | The one-shot that registers the demo patients with FerroPIX (ITI-104) and answers their pseudonyms (ITI-83); the deploy script's `identities` verb runs it | baked into the image; the compose file mounts the image into the one-shot, so `deploy.sh` never handles it |
| `sandbox-ferropix-rollout.sh` | The operator's script for the FerroPIX rollout on both boxes (below) | never. The operator runs it from a checkout |
| `env.example` | A copy-to-`.env` template: the DB box's private-network DSNs, FerroPIX's sealing key, the image references and the FerroTERM index list | never. `.env` is the operator's file, written by hand on the box (the rollout script writes the FerroPIX lines) |
| `cloud-init-postgres.yaml` | The DATABASE box, fresh to serving state: the `deploy` user, key-only SSH, both firewalls, Docker, and the `ferroehr-postgres` compose posture bound to the private address | the DB server's user data at creation, once |
| `ferroterm-index.env` | The record of the terminology index the box carries: its artifact layout, the `ferroterm-build` version and the editions. `scripts/checks/ferroterm-index-layout.sh` refuses a FerroTERM pin in `docker-compose.yml` whose layout differs | never. The index is built off-box and copied by the operator (below), and this file changes in the same pull request as the pin |
| `docker/terminology/seed/` (in the repository) | The licence-free shaped code systems FerroTERM serves on the box | baked into the ferroehr image at `/opt/sandbox-posture/terminology/`; the compose file mounts it into FerroTERM straight from the image, so `deploy.sh` never handles it |

The box holds **no checkout of this repository** and fetches nothing from it
over the network: the posture travels inside the published ferroehr image, so
a posture change arrives at a release, with that artifact's provenance. The
`hosted-deploy-script` CI job extracts `deploy.sh` out of `cloud-init.yaml`
and shellchecks it, and checks that `docker/Dockerfile` bakes every posture
path the script reads and every path the compose file reads out of the image.

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
   holds), `restart` (recreate the CDR so a fresh boot re-runs the
   migrations against the wiped database) and `identities` (below).

The deploy key is restricted on the box to `deploy.sh` (`command=` in
`cloud-init.yaml`), which reads `SSH_ORIGINAL_COMMAND` and accepts exactly
`deploy`, `restart`, `wipe` and `identities`. The lane builds nothing; it asserts by digest that
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

The reseed seeds the release the sandbox runs, with that release's scripts
(#4370). Its `identities` job reads `solution_version` from the ITS-REST
system `OPTIONS /ferroehr/rest/openehr/v1`, and both the `identities` and the
`seed` job check out the tag `v<version>`, so the routes the walker calls,
the corpus it loads and the manifest's pins are the deployed version's. A
change to the walker or the dataset on `main` reaches the sandbox with the
next release, and a version with no tag fails the run by name.

### The patients and FerroPIX

The CDR resolves patients through FerroPIX (`[ehr_index] provider = "ihe"`),
so every `EHR_STATUS` names its patient by a pseudonym in the clinical domain
`urn:oid:2.999.7.1` (`[privacy] subject_namespaces`). The manifest's subjects
(`sandbox-patient-01` and on, and the marker) are the patients' identifiers in
the synthetic hospital domain `urn:oid:2.999.1.1`. Before the seed, the
reseed's `identities` job pipes them over SSH into the deploy script's
`identities` verb, which runs the compose file's `ferropix-seed` one-shot:
each subject is fed to FerroPIX as the Patient Identity Source
`sandbox-source` (ITI-104, an idempotent conditional update), and asked for
as the CDR's client `ferroehr` (ITI-83), which mints its pseudonym on the
first ask and answers the same one after. The answer is the file
`reseed.sh` reads as `SANDBOX_IDENTITIES`. FerroPIX's database is never wiped,
so every night answers the same pseudonyms, and after a reset the CDR's EHRs
and FerroPIX's identities agree. A visitor sees the resolution through
`POST /ferroehr/rest/extensions/v1/query/cohort` with `subjects` named in
`urn:oid:2.999.1.1` (`[cohort] subjects = true`; the small-cell floor of 5
applies).

FerroPIX and the CDR accept a plain-`http` token issuer only on loopback
(RFC 8414 §2), so the development issuer (NAV's `mock-oauth2-server`, which
signs any client credentials request) owns one network namespace that the
CDR, FerroPIX and the seed join, as in the compose quickstart. The issuer
publishes no port and FerroPIX binds `127.0.0.1:8090`, so nothing outside the
box reaches either. A namespace owner with members cannot be replaced, so
`deploy.sh` removes the CDR and FerroPIX containers before its `up -d`.

One run loads, in about 590 requests:

| What | How much |
|---|---|
| ADL 1.4 operational templates | 17 (the curated CKM pack, `corpus/templates/ckm/`, plus one terminology-binding template) |
| ADL 2 archetypes | 233: 230 from the vendored 2013 CKM corpus plus 3 of our own |
| ADL 2 templates | 5 (4 of our own plus one carried by the corpus); two of ours are SOURCE templates the CDR flattens against that archetype library |
| EHRs | 8, each with a distinct subject and a mixed record |
| Compositions | 183 (167 ADL 1.4, 16 from CDR-generated ADL 2 examples) |
| Further versions | 6 COMPOSITION, 2 EHR_STATUS |
| Demographic parties | 6 persons, 3 roles, 2 party relationships |
| Directories | one FOLDER tree per EHR, its items real composition references |
| Stored AQL queries | 5, under the `eu.ferroehr.sandbox` namespace |

The ADL 2 archetype step offers the whole 322-file corpus, parents before
children, and pins both outcomes in the manifest: 231 artefacts stored, 91
refused with a `422`. 89 refusals are AOM2 validity findings on the 2013
conversions: constraints on attributes the RM does not declare
(`DV_QUANTITY.property`, and the RM functions `is_integral` and `offset` the
machine-readable RM omits), languages, slots and node ids that do not conform
to the flat parent, and eight children of refused parents that therefore have
no flat parent to validate against. Every refused file is adjudicated by name
in the corpus gate (`crates/openehr-adl/tests/it/ckm_archetype_packs.rs`), and
a change in either count fails the run instead of passing quietly.

The other two refusals need the LOINC index FerroTERM serves on the box:
`braden_scale` binds the mistyped LOINC answer code `LA9605-4` (VETDF), and
`braden_scale-child` then has no stored parent (VASID). A terminology server
that serves no LOINC cannot verify the binding, so the CDR warns and stores
both (AOM2 VETDF: "subject to tool accessibility"), and the library loads 233
/ 89. The seed asks the CDR for a LOINC code before the library step and
stops with that diagnosis when the posture serves none (#4278). A local run
without LOINC sets `SANDBOX_TERMINOLOGY=without-loinc` and expects 233 / 89.

Two of the eight EHRs end the run with
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
design: copy this directory's five posture files (`docker-compose.yml`,
`Caddyfile`, `ferroehr.sandbox.toml`, `ferroehr-viewer.sandbox.toml`,
`ferropix.sandbox.toml`) plus a filled-in `.env` to
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
reach the box. Step 4 of the rollout script below checks the layout and, given
`SNOMED_RF2_ZIP` and `LOINC_ZIP` on the operator's machine, runs exactly these
steps.

FerroTERM 0.3 reads its configuration tree only: the compose file sets
`FERROTERM__TERMINOLOGY__CODE_SYSTEMS` and `FERROTERM__TERMINOLOGY__INDEX`,
and the index list in `.env` is `FERROTERM_INDEXES`, comma-separated, where
FerroTERM 0.1 read `FERROTERM_INDEX`, colon-separated. FerroTERM 0.3.1 reads
artifact layout 7, the layout of the index the box carries (built with
`ferroterm-build` 0.1.6, `ferroterm-index.env`), so the move from 0.1.6 needs
no rebuild.

## Rolling out FerroPIX (#4242)

`sandbox-ferropix-rollout.sh` carries every hand step on both boxes. Run it
from a checkout on your own machine; it reaches the boxes over SSH as
`deploy` (`APP_HOST`, default `167.233.172.220`; `DB_HOST`, default
`postgres.ferroehr.eu`; `SSH_USER`). Every step prints `==> step N:` and then
`OK:`, `SKIP:` or `FAIL:` with its evidence, a re-run is safe, and `--check`
verifies without changing anything; with no `--step` it runs steps 1 to 6.
No secret is printed: the log shows a 12-digit SHA-256 fingerprint.

1. **Preflight.** Both boxes answer over SSH, and the log lists what runs on
   each (image, version, health).
2. **DB box.** Creates the `ferropix` role (no superuser) and database,
   revokes `PUBLIC`'s grants on it, and grants the role `CONNECT`, `CREATE`
   and `TEMPORARY`. The password is generated on the box (or read from
   `FERROPIX_DB_PASSWORD`) and kept in `/opt/ferroehr-postgres/.env` as
   `FERROPIX_DB_PASSWORD`. It checks that PostgreSQL is published on the
   private address alone, then logs in as `ferropix` from the app box over
   the private network.
3. **App box.** Writes `FERROPIX_DATABASE_URL`, `FERROPIX_DATA_KEY`
   (generated on the box) and `FERROTERM_INDEXES` (the old `FERROTERM_INDEX`
   with commas) into `.env`, keeping `.env.prev`. Per the lesson above, a line
   stays commented out until the running release knows it, unless no
   container of the installed posture receives the name. These three are
   compose substitution names that no container receives, so they are written
   active: the v5.5.0 deploy refuses a posture whose FerroPIX names are unset,
   and would otherwise never install. It comments out a `FERROTERM_IMAGE`
   line that names another FerroTERM than the compose file pins: the box
   carried FerroTERM 0.1.6 that way, which reads none of the 0.3 settings,
   so it served no index and no LOINC (#4278). It also installs this checkout's
   `deploy.sh`, which carries the `identities` verb the nightly reseed calls
   from the moment this change merges.
4. **The FerroTERM index.** Compares each index's `storeLayout` on the box
   with `ferroterm-index.env`; layout 7 on both means no rebuild.
5. **After the v5.5.0 deploy** (a `SKIP:` before it). Recreates the stack
   (`docker compose pull`, then the CDR and FerroPIX removed and `up -d`,
   never `restart`), or installs the posture with the `deploy` verb if the
   release's deploy did not. It waits for every container to report healthy,
   checks that FerroTERM runs the pinned version and that the CDR resolves the
   manifest's LOINC probe code, registers the demo patients through the
   `identities` verb, and, unless the demo data is already seeded under the
   pseudonyms, wipes and restarts the CDR before it reseeds (a definition an
   unfinished seed stored answers `409` and would count as stored). It then
   verifies end to end: a cohort named by the hospital identifiers
   resolves through FerroPIX to the EHRs carrying FerroPIX's pseudonyms,
   FerroTERM answers through the CDR's `/terminology` route, and the public
   URL serves the CDR and the viewer.
6. **The database box onto the pinned image** (#4252). Reads the running
   image, `server_version` and the data volume, and stops when the box
   already runs the pin. Otherwise it pulls the pin and refuses when its
   `PG_MAJOR` differs from the running major (a major upgrade needs
   `pg_upgrade`, which this step never runs). It then sets
   `FERROEHR_POSTGRES_IMAGE` in `/opt/ferroehr-postgres/.env` and points the
   compose image line at it, keeping `.env.prev` and
   `docker-compose.yml.prev`, and recreates PostgreSQL with
   `docker compose pull` and `docker compose up -d`, never `restart`. The
   sandbox is unavailable for about a minute. It waits for PostgreSQL to
   report healthy, checks that the container kept its data volume and its
   major, that the CDR answers `/health/readiness`, and that `ferropix` logs
   in from the app box over the private network.

Run steps 1 to 4 when this change merges, before the v5.5.0 release, and the
whole script again after the release's `sandbox` job.
Step 6 runs whenever the pin moves, and it is the one step that interrupts
the sandbox.

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
or credential beyond the two database DSNs and FerroPIX's sealing key exists
on the host. FerroPIX's database is the one store the nightly wipe leaves
alone: its patients and their pseudonyms are what the reseed writes into the
CDR, so they stay. **No backups**, deliberately.
