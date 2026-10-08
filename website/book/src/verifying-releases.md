# Verifying releases

Before you run a FerroEHR binary, image, or chart in a clinical environment,
you should be able to answer two questions about the bytes you downloaded:
**did they arrive intact**, and **were they built by this project's own release
pipeline**. This page is the operator's procedure for both: one command per
artifact kind, plus the deliberately-failing runs that prove your verification
is actually checking something.

Every release publishes signed build provenance, a dependency SBOM, and a plain
checksum alongside each artifact. Provenance is signed through
[Sigstore](https://www.sigstore.dev/), so you can verify it yourself, and the
signer identity is one you can pin to a single hardened workflow.

The repository moved to the `FerroHEALTH` organization on 2026-10-01, and
v4.3.2 is the first release built there. Releases up to and including v4.3.1
were built and signed while it was `rubentalstra/FerroEHR`, so their
attestations name that repository: verify them with `-R rubentalstra/FerroEHR`
and `--signer-workflow rubentalstra/FerroEHR/…`. Their images and charts were
published under `ghcr.io/rubentalstra`. The copies of v4.3.1 under
`ghcr.io/ferrohealth` carry the same digests, so `gh attestation verify` accepts
them with the old repository name. Their cosign signatures exist only beside
the originals, so run `cosign verify` against the `ghcr.io/rubentalstra` path.

<!-- toc -->

## What a release publishes

Substitute the release tag you downloaded for `<tag>` (for example
`v4.3.6`) and the architecture for `<arch>` (`x86_64` or `aarch64`) throughout
this page. Linux is the only published target.

| Asset | What it is |
|---|---|
| `ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz` | the stripped server binary |
| `…tar.gz.sha256sum` | a plain checksum of that tarball |
| `…tar.gz.sigstore.json` | the Sigstore bundle for its build-provenance attestation |
| `…tar.gz.sbom.sigstore.json` | the Sigstore bundle for its SBOM attestation |
| `…tar.gz.intoto.jsonl` | the same provenance as one DSSE-wrapped in-toto statement per line |
| `ferroehr-<tag>-<arch>-unknown-linux-gnu.cdx.json` | the CycloneDX dependency SBOM for that binary |
| `ferroehr-<tag>.spdx.json` | the SPDX SBOM of the source tree at the release commit |
| `ferroehr-<tag>-vulnerability-record.json` | every finding of a full scan of each image and binary, each with the OpenVEX statement that judges it ([the record](#the-exploitability-record)) |
| `ferroehr-<tag>-trivy-reports.tar.gz` | the raw Trivy reports the record was joined from |
| `<crate>-<crate-version>.cdx.json` | the CycloneDX SBOM of each published `openehr-*` crate |
| `<crate>-<crate-version>.crate.sbom.sigstore.json` | the Sigstore bundle binding that SBOM to the `.crate` archive crates.io serves |
| `docker-compose.yml` + the Keycloak and observability overlays | the quickstart stack, so a downloader never has to clone the repository |

Container images and the Helm chart are published to GHCR by their own lanes and
carry their own attestations; see [images](#a-container-image) and
[the chart](#the-helm-chart) below.

> [!NOTE]
> The release lane refuses to publish a release whose asset set is incomplete.
> It creates the release as a draft, attaches every asset, checks the full
> expected set is present, and only then publishes, because a published release
> is immutable on this repository, so a missing asset could never be added
> afterwards. The remedy for a bad cut is a new patch version, never a retag.

## A release binary

**The floor: the checksum.** Every tarball ships a `.sha256sum` beside it, and
this is the only verification available with neither `gh` nor `cosign` installed
, which is a realistic constraint in a locked-down clinical environment:

```bash
sha256sum -c ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.sha256sum
```

The file records the bare filename, so this works in whatever directory you put
the two files in. On macOS, `shasum -a 256 -c` is the same check.

> [!WARNING]
> Be clear about what a checksum buys: it detects a **corrupt or truncated
> download**, not a substituted release, because anyone who could replace the
> tarball could replace the checksum beside it. Only the Sigstore bundle answers
> "who built this". The checksum is a floor, not a substitute.

**The real check: the provenance attestation.**

```bash
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  -R FerroHEALTH/FerroEHR
```

**Without reaching GitHub.** Each release also carries its Sigstore bundles as
assets, so verification needs nothing but the artifact and the bundle, useful
on an air-gapped host, and the only form in which the signature travels with the
download:

```bash
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  --bundle ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.sigstore.json \
  --repo FerroHEALTH/FerroEHR
```

The `*.sbom.sigstore.json` asset beside it is the same thing for the SBOM
attestation, so "which dependency graph was this binary built from" is
verifiable offline too. Verifying an SBOM attestation needs one extra flag:
`gh attestation verify` enforces the SLSA provenance predicate by default, so
a non-provenance attestation must name its predicate type explicitly:

```bash
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  -R FerroHEALTH/FerroEHR \
  --predicate-type https://cyclonedx.org/bom
```

**Fully offline**, with no call to GitHub at all: fetch the trusted key
material once on a connected machine and carry it across with the bundle:

```bash
gh attestation trusted-root > trusted_root.jsonl        # on a connected host
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  --bundle ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.sigstore.json \
  --custom-trusted-root trusted_root.jsonl \
  --repo FerroHEALTH/FerroEHR
```

**Require the hardened signer.** Without a signer constraint you are trusting
that *some* workflow in this repository signed the artifact. Pin the one that
actually did:

```bash
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  -R FerroHEALTH/FerroEHR \
  --signer-workflow FerroHEALTH/FerroEHR/.github/workflows/release-build.yml
```

That workflow is the reusable release-build lane described under
[SLSA levels](#what-slsa-level-each-artifact-reaches) below.

**The binary also describes itself.** From v4.0.1 on, the shipped binary
embeds its own compressed dependency list in a `.dep-v0` linker section
(built with [`cargo auditable`](https://github.com/rust-secure-code/cargo-auditable);
the section is allocated, so it survives the release lane's `strip`). A
scanner that reads binaries — syft, trivy, grype, osv-scanner — recovers the
crate graph from the artifact itself, even when the release page's SBOM never
travelled with it:

```bash
syft ferroehr -o cyclonedx-json   # the extracted binary, not the tarball
```

From v4.3.5 on, the binaries inside the `ferroehr` and `ferroehr-viewer`
images are built with `cargo auditable` as well, so a scan of either image
lists the Rust dependencies of the binary it carries, and the
[exploitability record](#the-exploitability-record) judges them per image.
Images of earlier releases carry no such list: a scan of them sees the
operating-system layer only, and the server's Rust dependencies are visible
through the tarball scan and its SBOM.

```bash
# <version> is v4.3.5 or later, without the v
trivy image --list-all-pkgs --format json ghcr.io/ferrohealth/ferroehr-viewer:<version> \
  | jq -r '.Results[] | select(.Type == "rustbinary") | .Packages[] | "\(.Name) \(.Version)"'
```

> [!TIP]
> `gh attestation verify` reports success by **exiting zero and printing
> nothing** in current `gh` versions. Check the exit status in scripts rather
> than grepping for a success message.

## Prove your verification can fail

A check that cannot fail proves nothing, so do both of these once, by hand,
before you trust a passing run:

```bash
# 1. Tamper with the artifact — verification must refuse it.
printf 'x' >> ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz
gh attestation verify ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz \
  --bundle ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.sigstore.json \
  --repo FerroHEALTH/FerroEHR
# → Error: verifying with issuer "sigstore.dev"; exit status 1

# 2. Name a repository that did not build it — also non-zero.
gh attestation verify … --repo someone-else/something
```

Re-download the tarball afterwards. Only once you have seen both refusals does a
passing run mean something.

**Cross-check the digest.** The same digest is produced independently in three
places, so they are worth comparing against each other rather than trusting any
one of them:

```bash
# the published checksum file
cat ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.sha256sum

# the digest the provenance statement was signed over
jq -r '.payload' ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz.intoto.jsonl \
  | base64 -d | jq -r '.subject[].digest.sha256'

# the bytes on your disk
sha256sum ferroehr-<tag>-<arch>-unknown-linux-gnu.tar.gz
```

## A container image

The three images (`ferroehr`, `ferroehr-viewer`, `ferroehr-postgres`) each
carry a Sigstore-signed SLSA provenance attestation, plus the SPDX SBOM and
provenance the builder writes onto the image index itself.

```bash
gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:4.3.6 \
  -R FerroHEALTH/FerroEHR
```

> [!IMPORTANT]
> **Image tags carry no `v` prefix.** A release publishes `4.0.0`, `3.20` and
> `latest` (plus a `sha-…` tag per commit), while the release *assets* are named
> after the `v4.0.1` git tag. Using `v4.0.1` as an image reference will simply
> not resolve.

The development tags (`ghcr.io/ferrohealth/ferroehr:main` and its two
siblings) are signed the same way, so you can rehearse the command against them
before a release.

Add `--signer-workflow FerroHEALTH/FerroEHR/.github/workflows/build-image.yml`
to require the hardened image-build lane specifically rather than any
workflow in this repository.

**Verify by digest where it matters.** A tag is mutable, so anything that
GATES on verification (admission control, a deploy script) should resolve the
tag once and verify the digest it resolved — otherwise the bytes verified and
the bytes pulled can differ:

```bash
digest=$(docker buildx imagetools inspect ghcr.io/ferrohealth/ferroehr:4.3.6 \
  | awk '/^Digest:/{print $2}')
gh attestation verify "oci://ghcr.io/ferrohealth/ferroehr@${digest}" \
  -R FerroHEALTH/FerroEHR
```

The attestation is also pushed to the registry itself, so a host with
registry access but no GitHub API path can verify from the registry alone
with `--bundle-from-oci`. And the SBOM the builder wrote onto the image index
is readable without any verifier at all:

```bash
docker buildx imagetools inspect "ghcr.io/ferrohealth/ferroehr@${digest}" \
  --format '{{ json .SBOM }}'
```

Enforcing this at admission time in a Kubernetes cluster is a separate job with
its own machinery; see
[Images: build, provenance, scanning](installation/hardening-supply-chain.md).

## The Helm chart

The chart carries a keyless **cosign signature** in addition to its provenance
attestation. The two are not redundant: the attestation says what the chart was
built from, the signature says who signed the artifact you pulled, and the
signature is what Helm-ecosystem tooling looks for.

```bash
cosign verify ghcr.io/ferrohealth/charts/ferroehr:<chart-version> \
  --certificate-identity-regexp '^https://github\.com/FerroHEALTH/FerroEHR/\.github/workflows/build-chart\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Both flags are the point: without an identity and an issuer, `cosign verify`
accepts a signature from anyone in the transparency log.

```bash
gh attestation verify oci://ghcr.io/ferrohealth/charts/ferroehr:<chart-version> \
  -R FerroHEALTH/FerroEHR \
  --signer-workflow FerroHEALTH/FerroEHR/.github/workflows/build-chart.yml
```

> [!NOTE]
> **The chart version is not the product version.** The chart runs its own
> SemVer line, so a chart version and the `appVersion` it deploys move
> independently. Read the chart's `appVersion` to learn which server release it
> defaults to.

The chart ships **no** PGP `.prov` file, so `helm install --verify` does not
apply. That is deliberate: a `.prov` needs a long-lived private key in CI, and
keyless Sigstore signing gives a consumer a stronger identity to pin without
one.

The publishing lane refuses to overwrite a chart version that already exists,
reads both the signature and the attestation back from the registry before
reporting success, and checks that the `appVersion` image accepts the chart's
rendered defaults. It also attests the chart's SBOM, which names every
image the chart deploys by digest
([the SBOMs](#the-sboms-one-per-published-artefact)).

## The SBOMs, one per published artefact

Every artefact this project publishes carries a software bill of materials, and
each one answers a different question about a different download:

| Artefact | SBOM | Where | Format | Answers |
|---|---|---|---|---|
| the server binary | `ferroehr-<tag>-<arch>-unknown-linux-gnu.cdx.json` | a release asset per architecture, attested against the tarball (`…tar.gz.sbom.sigstore.json`) | CycloneDX 1.5 | *what is inside the binary I am about to run?* Every cargo component with a `pkg:cargo/…` purl and licence, most with checksums, and the **dependency edges**, so "is this crate direct or four levels down" is answerable. This is what a vulnerability scanner consumes. |
| each container image | the image SBOM | written onto each image index by the builder | SPDX | *what is in the image's OS layer?* Which is what matters for `ferroehr-postgres`, built on the upstream `postgres` image. |
| each `openehr-*` crate | `<crate>-<crate-version>.cdx.json` | a release asset per crate, attested against the `.crate` archive crates.io serves (`<crate>-<crate-version>.crate.sbom.sigstore.json`) | CycloneDX 1.5 | *what does this library pull into my build?* The crate's normal and build dependencies; dev-dependencies are left out, as `cargo package` strips them. |
| the Helm chart | the chart SBOM | an attestation on the chart's OCI digest in GitHub's attestation store | CycloneDX 1.5 | *which images does this chart deploy?* Each image the chart can deploy, named by repository, tag and the **digest** the tag resolved to when the chart was published, with its platforms. |
| the source tree | `ferroehr-<tag>.spdx.json` | a release asset per release | SPDX 2.3 | *what am I redistributing, and under what terms?* The attribution and licence-obligation view of the source tree at the release commit, the document a legal or procurement reviewer of a multi-licence redistribution asks for. |

The crate SBOMs ride the server release that ships them, so `<crate-version>`
is the crate's own version (the crates run their own SemVer line). A crate
version that did not change in a release is described again, against the
archive crates.io already serves. Verify a crate SBOM against the archive you
downloaded:

```bash
curl -sSLO https://static.crates.io/crates/openehr-rm/openehr-rm-<crate-version>.crate
gh attestation verify openehr-rm-<crate-version>.crate \
  -R FerroHEALTH/FerroEHR \
  --predicate-type https://cyclonedx.org/bom \
  --signer-workflow FerroHEALTH/FerroEHR/.github/workflows/release.yml
```

Read the chart SBOM, and verify it in the same step:

```bash
gh attestation verify oci://ghcr.io/ferrohealth/charts/ferroehr:<chart-version> \
  -R FerroHEALTH/FerroEHR \
  --predicate-type https://cyclonedx.org/bom \
  --signer-workflow FerroHEALTH/FerroEHR/.github/workflows/build-chart.yml \
  --format json --jq '.[].verificationResult.statement.predicate'
```

CycloneDX 1.5 is the highest version `cargo-cyclonedx` emits (it accepts 1.3,
1.4 or 1.5 and defaults to 1.3, so the release lane sets it explicitly); it
carries everything 1.6 consumers read. The release lane refuses an SBOM that
is not the format it claims or that names another artefact: the repository
SPDX document must name itself `ferroehr` and list at least one package, a
crate SBOM must describe that crate at that version, and every image in the
chart SBOM must resolve to a `sha256` digest before the chart is pushed.

## What SLSA level each artifact reaches

The levels are the [SLSA v1.2 Build track](https://slsa.dev/spec/v1.2/build-requirements)
(the current spec; v1.0 is marked Retired, and the Build-track requirements
are unchanged across those versions — GitHub's own documentation still
phrases the same construction against "SLSA v1.0"). They differ per artifact,
so a table is the honest form:

| Artifact | Level | Why |
|---|---|---|
| release binaries + their SBOM | **Build L3** | built and attested inside a *reusable* workflow (`release-build.yml`), so the signing material is out of reach of any caller-defined step |
| container images | **Build L3** | built, pushed and attested inside a reusable workflow (`build-image.yml`); the calling jobs pass names and label text, never steps |
| the Helm chart | **Build L3** | packaged, pushed, signed and attested inside a reusable workflow (`build-chart.yml`); the caller carries only triggers |

Images and the chart reach L3 from the first publish after v4.0.1; earlier
artifacts were attested at Build L2 (in the job that built them), and their
attestations verify with the repository-scoped command, not the pinned
signer.

Build L3's distinguishing requirement is that secret material used for
authenticating the provenance "MUST NOT be accessible to the environment
running the user-defined build steps". Every step of a GitHub Actions job shares one runner VM, so
attesting inside the building job cannot satisfy it. The release lane therefore
builds and signs inside a reusable workflow: it runs on its own VM, and a caller
passes declared inputs; it cannot add steps. The calling job has no steps at
all, which is what makes the property hard to lose by accident, and it is why
`--signer-workflow` is worth passing.

**What is still not claimed**, in either lane: the isolation is GitHub's rather
than this project's, and nothing here asserts a reproducible or hermetic build;
those are separate SLSA tracks this project does not address. Provenance proves
*where* an artifact was built, not that the source was good. Naming the boundary
is worth more than rounding a level up.

## The `openehr-*` crates

The nine specification crates publish to crates.io through Trusted
Publishing: the workflow authenticates with a short-lived OIDC token, and no
long-lived crates.io token exists anywhere. Be precise about what that does
and does not give you: it is **authentication, not provenance**. crates.io
records and displays nothing about how a crate was built, and the Rust RFC
that introduced Trusted Publishing lists provenance verification as
explicitly out of scope — there is no registry-side attestation to verify,
from this project or any other. `cargo` checks the crate's checksum against
the registry index, and that is the whole registry-side story. The crates'
source of truth is this repository: the publishing lane, the tags, and the
public history.

What this repository does attest is each crate's SBOM, against the very
`.crate` archive crates.io serves; the crates.io leg of the release reads that
attestation back from crates.io's own copy after publishing. See
[the SBOMs](#the-sboms-one-per-published-artefact) for the command.

## Findings a scanner will report

Run a scanner over a FerroEHR artifact and it will report findings. Every one
this project has assessed and accepted is published as an
[OpenVEX](https://openvex.dev) document under
[`security/vex/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/security/vex),
carrying a controlled-vocabulary justification and an impact statement you can
check, rather than an ignore entry that records only the verdict. Point your
tooling at them (`trivy --vex`, and most SCA platforms take an OpenVEX feed).

| Document | Covers |
|---|---|
| `rust-advisories.openvex.json` | the Rust dependency advisories: the ones the advisory gate accepts, plus one that only a `Cargo.lock`-reading scanner reports |
| `ferroehr-os.openvex.json` | the Debian packages the `ferroehr` and `ferroehr-viewer` images inherit from their distroless base (glibc, the GCC runtime libraries, zlib) |
| `postgres-os.openvex.json` | the Debian packages of the `ferroehr-postgres` image, one statement per finding |

A statement with status `affected` names a finding that applies to you, with
an action statement saying what to do until the fix ships. `trivy --vex`
leaves those findings in its output on purpose; only `not_affected` and
`fixed` statements remove one.

The `ferroehr-postgres` image is the upstream `postgres:18.6` image with
Debian's security updates applied at build time, minus two things: GnuPG and
the libraries only it uses (upstream needs it only to import a package signing
key while building the image), and upstream's `gosu`, a Go binary, which a
short shell wrapper around util-linux `setpriv` replaces under the same name.
Every finding that remains is a Debian package with no fix in Debian yet. Six
are `affected`: three libxml2 findings and two glibc character-converter
findings, which any database login reaches through PostgreSQL's XML functions,
and the resolver finding CVE-2026-8674. FerroEHR sends no XML to the database,
so the action statements come down to giving database logins only to the
FerroEHR roles and your administrators, and keeping the resolver configuration
the container receives under your control.

The Rust document is **generated** from `deny.toml` (the gate that actually
decides whether a build passes) joined with the published reasoning, and a CI
job fails if the two disagree in either direction. So an advisory cannot be
accepted without a justification reaching you, and a justification cannot claim
something the gate does not do.

One asymmetry worth knowing, because it produces findings that are real reports
of nothing. `Cargo.lock` records the union of every dependency any feature
combination *could* pull, so a scanner reading the lock file alone reports
crates this project's feature set never compiles. `cargo deny` resolves features
and does not. Where the two disagree in that direction the feature-resolving
tool is the more precise instrument, and the VEX document carries that argument
for the specific crate it applies to, so you do not have to take our word for it
in prose.

The published images are additionally re-scanned on a weekly schedule at the
tag an operator actually pulls. A fixable high-or-critical finding files or
updates one tracker issue, and the issue is the alert: the scheduled run itself
stays green, because a red scheduled run nobody looks at is not a control.

### The exploitability record

The scan that gates an image blocks only high and critical findings that have
a fix. Every release therefore also scans each image (`ferroehr`,
`ferroehr-viewer`, `ferroehr-postgres`, both platforms, at the digest the
release pushed) and each server binary (`x86_64` and `aarch64`) with **no
severity floor and unfixed findings included**, and publishes the result as
two release assets:

- `ferroehr-<tag>-trivy-reports.tar.gz`: the raw Trivy JSON reports, one per
  image and platform and one per binary.
- `ferroehr-<tag>-vulnerability-record.json`: every finding from those
  reports, joined with the OpenVEX statement under `security/vex/` that judges
  it. Each entry names the product (`pkg:oci/<image>` or
  `pkg:cargo/ferroehr-server`), the vulnerability id, the package and its
  installed version, the fixed version where one exists, the severity, and the
  statement: its status (`not_affected`, `affected` or `fixed`), its
  justification, impact statement or action statement, and the document it
  comes from.

The release is refused while any finding has no such statement: the record is
checked before the images are tagged, so a refused release leaves its images
untagged and its GitHub release a draft. The rule for a finding with a fix is
to fix it by rebuilding, not to judge it. A statement counts only when it is a judgement:
`under_investigation` does not, and neither does `not_affected` without a
justification or impact statement, or `affected` without an action statement.

Read what a release shipped and why:

```bash
gh release download <tag> -R FerroHEALTH/FerroEHR -p 'ferroehr-*-vulnerability-record.json'
jq -r '.records[] | [.product, .id, .severity, .statement.status, .statement.justification] | @tsv' \
  ferroehr-<tag>-vulnerability-record.json | sort -u
```

## If verification fails

A failing verification is a security report, not a support question. Do not run
the artifact, and follow
[SECURITY.md](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md):
report privately, never as a public issue. Note that only the newest release
receives fixes; there is no maintenance branch to backport to.
