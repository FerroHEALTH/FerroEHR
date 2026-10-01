# Images: build, provenance, scanning

The image half of the audit. Building a minimal image and signing it is **ours**;
requiring a cluster to *check* that signature before running the image is
**yours**, and this page carries the copyable policies for both engines that can
do it.

<!-- toc -->

## The build phase, and what distroless costs

**Ours.** Each build-phase control and what actually satisfies it:

| Control | Satisfied by |
|---|---|
| Minimal image (distroless) | `gcr.io/distroless/cc-debian13:nonroot`: no shell, no package manager, no libc tooling |
| Image currency | base images and CI job containers pinned **by digest**, not tag, so a rebuild cannot silently change bytes |
| Vulnerability identification in CI | Trivy over every published image, hadolint over every Dockerfile, plus secret and misconfiguration scanning over the whole tree |
| Continuous scanning after release | a scheduled scan of the *published* tags; [below](#continuous-scanning-of-published-images) |
| Authorized images only | signed provenance published; **enforcement is the operator's**; [below](#image-provenance-at-admission) |
| Non-root by construction | the image declares `USER 65532:65532` (numeric, so the kubelet can verify it without reading `/etc/passwd`), and the pod pins `runAsNonRoot` plus uid 65532 independently |

Three images are published, and they are not equivalent in risk. The server and
the viewer are distroless and carry almost no OS package surface. The
PostgreSQL image is a thin, `COPY`-only layer over the upstream `postgres` image
(it adds initialization scripts and nothing else) so its package set is
upstream's, and its CVEs arrive on upstream's schedule rather than ours. The chart
deploys **none** of the second and third: it takes an external DSN and can
optionally render the viewer.

**What distroless costs, stated before an incident rather than during one: there
is no shell in the image, so `kubectl exec … -- sh` does not work.** That is the
security property working as intended (an attacker who achieves command
execution finds no interpreter, no `curl`, no package manager) but it changes how
you debug. Use instead:

- `kubectl logs` (the server logs JSON by default, for a collector),
- the always-on `/health/readiness` body, which names the failing dependency,
- `/management/*` for the effective configuration, live log filters and an
  on-demand CPU flamegraph,
- `kubectl debug -it <pod> --image=busybox:1.37 --target=ferroehr` starts an ephemeral
  container shares the target's namespaces without adding a shell to the image
  that ships.

The registry posture: the images are **public** on GHCR, so a pull needs no
credential and there is nothing to leak. Public does not mean trusted, which is
the point of the next section: nothing about a public registry stops a cluster
pulling a *different* image with the same name from somewhere else.

## Image provenance at admission

**The operator's, and this is provenance nobody in your cluster currently
checks.**

The publishing lanes attest their artifacts through **keyless Sigstore**, so a
verifier can establish that an artifact came from this repository's build: the
published images carry a signed SLSA v1 provenance attestation, and the chart
carries both an attestation and a cosign signature over its digest. Nothing in a
cluster *requires* that check before running an image, and a signature nobody
verifies changes nothing about what actually runs. That is what the policies below
close.

### The identity to trust, read off a real artifact

Each lane signs with a short-lived Fulcio certificate whose **subject alternative
name is the workflow that issued the token**, and whose issuer is GitHub's OIDC
provider. Read it off a published artifact rather than deriving it from a workflow
file:

```shell
gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:main \
  -R FerroHEALTH/FerroEHR --format json \
  | jq '.[0].verificationResult.signature.certificate
        | {subjectAlternativeName, issuer, sourceRepositoryRef, runnerEnvironment}'
```

```json
{
  "subjectAlternativeName": "https://github.com/FerroHEALTH/FerroEHR/.github/workflows/build-image.yml@refs/heads/main",
  "issuer": "https://token.actions.githubusercontent.com",
  "sourceRepositoryRef": "refs/heads/main",
  "runnerEnvironment": "github-hosted"
}
```

**The SAN's ref varies with the trigger, and that is the part a policy gets
wrong.** Each lane runs on more than one ref, so each issues more than one
identity:

| Artifact | Signing workflow | SAN on a release build | SAN on a development build |
|---|---|---|---|
| the three images | `build-image.yml` — the reusable builder both callers use | `…/build-image.yml@refs/tags/vX.Y.Z` (the release pipeline) | `…/build-image.yml@refs/heads/main` (via `containers.yml`) |
| the chart | `build-chart.yml` — the reusable chart lane | `…/build-chart.yml@refs/tags/vX.Y.Z` (the release pipeline's chart leg) | `…/build-chart.yml@refs/heads/main` (a `workflow_dispatch` chart-only publish) |
| the release binaries | `release-build.yml` | `…/release-build.yml@refs/tags/vX.Y.Z` | *(none; the lane only runs on a tag)* |

All three prefixed with `https://github.com/FerroHEALTH/FerroEHR/.github/workflows/`,
and all with issuer `https://token.actions.githubusercontent.com`.

The release binaries are signed by `release-build.yml` rather than by the release
workflow because the build lives in a **reusable** workflow: the certificate
names the workflow that owns the build definition, which is what makes the
`--signer-workflow` pin below meaningful.

**Pick the ref set deliberately, because the choice is a refusal.** A policy
matching `refs/tags/v…` only admits released images and **refuses
`ghcr.io/ferrohealth/ferroehr:main`**, correct for production, and the
reason a policy tested against `:main` appears broken when it is working. A
staging cluster that runs `:main` needs both refs. Nothing accepts an
arbitrary branch: `refs/heads/main` is exact, not a prefix match.

### Kyverno

The engine
[chosen here](hardening-network-policy.md#centralized-policy-and-which-engine).
Two details decide whether this policy works at all:

- **`type: SigstoreBundle`.** These attestations are GitHub Artifact
  Attestations, stored in the [Sigstore bundle
  format](https://docs.sigstore.dev/about/bundle/) as an OCI referrer. Kyverno
  reads that format only under this type; the field
  [defaults to `Cosign`](https://kyverno.io/docs/policy-types/cluster-policy/verify-images/sigstore/),
  which looks for a `sha256-<digest>.sig` tag that these images do not have (it
  returns 404: the bundle is a referrer, not a cosign tag). Requires
  **Kyverno 1.13 or newer**.
- **`attestations:`, not `attestors:` alone.** Kyverno's own rule is that
  "each `verifyImages` rule can be used to verify signatures or attestations,
  but not both", and what the image lane produces is a signed *attestation*:
  there is no detached image signature. A rule with `attestors:` at the top level
  therefore fails **closed** on a perfectly legitimate image.

```yaml
apiVersion: kyverno.io/v1
kind: ClusterPolicy
metadata:
  name: ferroehr-image-provenance
  annotations:
    pod-policies.kyverno.io/autogen-controllers: none
spec:
  background: false
  webhookTimeoutSeconds: 30
  rules:
    - name: verify-ferroehr-provenance
      match:
        any:
          - resources:
              kinds: [Pod]
              namespaces: [ferroehr]
      verifyImages:
        - imageReferences:
            - "ghcr.io/ferrohealth/ferroehr"
            - "ghcr.io/ferrohealth/ferroehr:*"
            - "ghcr.io/ferrohealth/ferroehr-viewer*"
          # Sigstore bundle format, GitHub Artifact Attestations. Omitting
          # this defaults to Cosign, which looks for a signature that does not
          # exist and refuses every image.
          type: SigstoreBundle
          failureAction: Enforce
          attestations:
            - type: https://slsa.dev/provenance/v1
              attestors:
                - count: 1
                  entries:
                    - keyless:
                        issuer: https://token.actions.githubusercontent.com
                        # Released images only. For a staging cluster that runs
                        # the development tag, make the group
                        # `(heads/main|tags/v.+)`.
                        subjectRegExp: '^https://github\.com/FerroHEALTH/FerroEHR/\.github/workflows/containers\.yml@refs/(tags/v.+)$'
                        rekor:
                          url: https://rekor.sigstore.dev
              conditions:
                - all:
                    - key: '{{ buildDefinition.buildType }}'
                      operator: Equals
                      value: https://actions.github.io/buildtypes/workflow/v1
```

Add `ghcr.io/ferrohealth/ferroehr-postgres*` to `imageReferences` only if you run
that image in the namespace; the chart never installs it.

`failureAction` sits on the `verifyImages` entry: the spec-level
`validationFailureAction` is deprecated in the CRD ("use `validationFailureAction`
under the validate rule instead"), and a `verifyImages` rule has no validate
block. `mutateDigest`, `verifyDigest` and `required` all default to `true`, which
is what you want: a tag is rewritten to the digest that was verified, and an image
with no attestation is refused rather than passed.

### sigstore-policy-controller

If you already run it. The equivalent two details:

- **`signatureFormat: bundle`** on the authority. The default is `legacy`
  (cosign's own), which cannot read these attestations. Requires
  **policy-controller v0.13.0 or newer**.
- **an `attestations:` entry**, because in bundle format policy-controller
  supports only attestations, not plain signatures.

```yaml
apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: ferroehr-image-provenance
spec:
  images:
    - glob: "ghcr.io/ferrohealth/ferroehr**"
  authorities:
    - keyless:
        url: https://fulcio.sigstore.dev
        identities:
          - issuer: https://token.actions.githubusercontent.com
            # Same group as the Kyverno policy: `(heads/main|tags/v.+)` for a
            # staging cluster that runs the development tag.
            subjectRegExp: '^https://github\.com/FerroHEALTH/FerroEHR/\.github/workflows/containers\.yml@refs/(tags/v.+)$'
      signatureFormat: bundle
      attestations:
        - name: require-slsa-provenance
          predicateType: https://slsa.dev/provenance/v1
```

> [!NOTE]
> **What has been checked, and what has not.** The identity, the issuer and the
> predicate type above are verified first-hand against the published images with
> `cosign verify --certificate-identity-regexp …`, which admits them and refuses
> both an unsigned image and, under a tags-only pattern, a `:main` image, so
> the matcher is neither vacuous nor accidentally permissive. The manifests are
> checked field by field against the published `ClusterPolicy` and
> `ClusterImagePolicy` CRDs. **Neither policy has been exercised by a running
> admission controller**, and the Kyverno CLI is no substitute: `kyverno test`
> reports a `verifyImages` rule as `Excluded` and returns the same verdict
> whichever result you assert. Before enforcing, run the policy in `Audit`
> (Kyverno) or as a `warn` policy (policy-controller) long enough to see one real
> deployment pass.

**The chart is signed too, and verifying it is a release-time check rather than a
per-pod one**: an admission controller sees pods and images, never the Helm
artifact a human pulled. The chart's own commands are on the
[Kubernetes page](kubernetes.md#verifying-what-you-installed); its identity is the
`build-chart.yml` row in the table above.

**Should the chart ship an admission policy? No, and the reason is structural:** an
admission policy is cluster-scoped and governs workloads the chart knows nothing
about, while the chart deliberately renders no cluster-scoped object at all (see
[namespaces](hardening-network-policy.md#namespaces-and-one-instance-per-organisation)). A
`ClusterPolicy` in this chart would mean `helm uninstall` removing a control that
other releases had come to depend on. The chart's contribution is the policy
*document*, here, versioned with the lanes whose identity it encodes.

**Without an admission controller**, the manual equivalent is a release-time check.
Add `--signer-workflow` to insist on the lane as well as the repository; without
it you are trusting that *some* workflow here signed the image:

```shell
gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:main \
  -R FerroHEALTH/FerroEHR \
  --signer-workflow FerroHEALTH/FerroEHR/.github/workflows/build-image.yml
```

Substitute a `vX.Y.Z` tag for `main` on a release; the signer workflow is the
same.

> [!IMPORTANT]
> Signing landed in the publishing lanes during the `3.17.4` cycle, so
> image tags from before it answer `HTTP 404: Not Found`, and there is
> nothing to verify, which is the correct answer and not a verification failure to
> work around. Pin a current version instead.

Then deploy **by digest** (`image.digest`), so what you verified is what runs: a
tag can be moved afterwards, a digest cannot.

## Continuous scanning of published images

**Ours.** CI scans images at build time, which catches what was known when they
were built and nothing after. A CVE published the week after a release applies to
the image people are running, so the published tags are re-scanned on a weekly
schedule: all three images at the tag a user pulls, with the same severity floor
and the same adjudicated exceptions as the build-time scan, and the OpenVEX
documents applied so an accepted finding stays accepted with its argument
attached.

A finding does two things, because either alone fails quietly: it opens (or
comments on) a tracking issue, **and** it fails the run. A red scheduled run
nobody looks at is not a control, and an issue with no failing check can be closed
without the finding being addressed.

The PostgreSQL image is what this lane exists for, and it has already fired: the
`3.17.5` release moved that image onto a rebuilt PostgreSQL 18.4 base to pull in
current Debian packages, changing nothing about FerroEHR itself. The distroless
images carry almost no OS package surface, and when the lane was written none of
the three published images carried a fixable HIGH or CRITICAL finding.

The remediation side is standing machinery, not a per-incident scramble. The
PostgreSQL image applies Debian security updates at its own build time, so an
OS-package fix published between upstream base rebuilds reaches the image at
the next release rather than waiting on someone else's cadence; a second
weekly lane watches the pinned base itself and opens an issue when a newer
PostgreSQL patch tag exists or the pinned tag is re-pointed upstream; and the
exact published-image scan is reproducible locally against a rebuilt
candidate, so a fix is proven clean before it merges.

## The supply-chain map

Each cheat-sheet supply-chain control, and the artifact that satisfies it, so a
reader can check rather than trust:

| Control | Satisfied by | Check it yourself |
|---|---|---|
| Trusted, minimal base images | `gcr.io/distroless/cc-debian13:nonroot`, **digest-pinned**; build stages pinned by digest too | `grep FROM docker/Dockerfile` |
| Vulnerability scanning in CI | Trivy over every published image, HIGH/CRITICAL with a fix available | the `image vulnerability scan` job log |
| Scanning after release | a weekly scan of the published tags | `.github/workflows/image-scan.yml` |
| Dockerfile linting | hadolint, with adjudicated exceptions in `.hadolint.yaml` | the `Dockerfile lint` job |
| Secret + misconfiguration scanning | Trivy's `secret` and `misconfig` scanners over the tree | the `tree-scan` job |
| Dependency advisories | `cargo deny` on every change, plus a scheduled latest-dependencies lane | `cargo deny check` |
| Signed images | a Sigstore keyless SLSA v1 provenance attestation per image | `gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:<tag> -R FerroHEALTH/FerroEHR` |
| Signed chart | an attestation plus a cosign signature over the chart digest, both read back from the registry before the lane reports success | `gh attestation verify oci://ghcr.io/ferrohealth/charts/ferroehr:<version> -R FerroHEALTH/FerroEHR` |
| SBOM | an SPDX SBOM written onto the image index by the builder; a CycloneDX dependency-graph SBOM attached **and Sigstore-attested** per released binary | `docker buildx imagetools inspect <image> --format '{{json .SBOM}}'` |
| Adjudicated findings carry their argument | OpenVEX documents under `security/vex/`, applied by the scheduled scan | read the `impact_statement` in the document |
| Secured CI/CD | every `uses:` digest-pinned, `permissions: {}` by default, no context interpolated into a shell, zizmor and CodeQL over the workflows themselves | the `zizmor` job |
| No long-lived registry token | crates.io Trusted Publishing (OIDC); GHCR uses the ephemeral workflow token | the `crates.io` leg of `.github/workflows/release.yml` |
| Independent grade | OpenSSF Scorecard, computed by someone other than us | the Scorecard badge |

**Two gaps remain, stated here rather than left out of a page that otherwise reads
as complete:**

1. **Nothing verifies the signatures at admission.** We sign; no cluster is
   required to check before running an image. The policies to close it are
   [above](#image-provenance-at-admission), and neither has yet been exercised by
   a running admission controller, which is why the instruction there is to run
   them in audit mode first.
2. **Provenance exists only from the `3.17.4` cycle onward.** Images published
   before the signing lane landed carry no attestation and never will, because a
   published artifact is not replaced. `gh attestation verify` on those returns a
   404, which is the correct answer and not a verification failure to work around.
