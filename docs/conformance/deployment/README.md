# Deployment-conformance records

The two deployment harnesses write their machine-readable records here — the
deployment analogue of what `docs/conformance/<sut>/` holds for the wire.

| Harness | Record | Asks |
|---|---|---|
| `scripts/deploy-probe.sh` | `compose.json` | does the software work when it is deployed? |
| `scripts/deploy-probe-k8s.sh` | `kubernetes.json` | does the cluster apply what the chart asked for? |

Those are different questions, and only the second is what an operator running
the chart actually gets. `helm template` proves the YAML; the API server, the
kubelet and the container runtime each get a say afterwards, and every one of
them can decline. So the Kubernetes probes read their answers from the runtime
spec, from admission, and from the EndpointSlice — never from the manifest we
wrote.

## In CI

The compose harness runs as the `deploy-probe` job on any change to the code,
the compose files, the server image or the harness itself. It reuses the binary
the `ui-e2e` lane already builds — packaged COPY-only in seconds, because
building the image again would double a long lane to prove the same bytes — and
uploads its record on every run, passing or failing. A failed run is exactly
when the per-probe layer attribution and the not-exercised ledger are worth
reading.

The **terminology** family does not run there. It needs a licensed SNOMED
release that may not be fetched onto a shared runner, and it is opt-in locally.

A record is a measurement of **one SUT**, so it is only meaningful with the
image it was taken against. The harness names that image in its output, and
the record carries the per-probe outcome plus the layer each failure indicts.

## Reading a record

```json
{"platform":"compose","passed":9,"failed":1,"uncovered":7,
 "probes":[{"id":"P-MM-EXPAND","state":"working","outcome":"fail",
            "layer":"server","issue":"#2197","title":"…"}],
 "not_exercised":[{"what":"kubernetes platform","why":"…"}]}
```

- **`state`** is one of `off` / `working` / `broken`. All three matter: two of
  the defects that prompted this instrument lived in states nobody exercised —
  a disabled integration that stranded data, and a dependency outage nothing
  probed.
- **`layer`** is what a failure indicts — `server`, `image`, `chart`,
  `compose`, or `docs`. A red row that says only "it did not work" costs more
  than it saves, so the layer is part of the record rather than left to the
  reader.
- **`issue`** marks a probe that reproduces a defect this project shipped. It
  must fail on the unfixed code and pass on the fixed code; that is what turns
  a one-off sweep into a permanent net.
- **`not_exercised`** is not an appendix. Silence read as coverage is how ten
  defects reached a release while a 4447-test suite stayed green, so every gap
  is declared in the artifact itself.

## Why records are not committed by default

Unlike the CNF baseline, these are not a ratchet: the record depends on which
image, overlays and profiles the run used, so a committed file would invite
comparison between runs that measured different systems. Commit one only
alongside the SUT identification that makes it readable.

## Running the terminology probe against real SNOMED CT

The terminology family drives FerroTERM, the terminology server FerroEHR ships
with, over a real SNOMED CT release, because a code system we seeded ourselves
would only prove our client can talk to our own fixture: `$subsumes` in
particular means nothing without a published hierarchy behind it. It uses the
quickstart's own terminology overlay (`docker-compose.terminology.yml`): the
overlay's one-shot `ferroterm-build` turns the archive into an index in a named
volume, and FerroTERM opens it, which is the path an operator takes.

**Local only, and the package is never in this repository.** The RF2 archive is
licensed content under a SNOMED International Affiliate agreement: it is not
committed, not fetched by CI, and not baked into an image. There is deliberately
no CI lane: a workflow that downloaded it onto a shared runner would be moving
licensed content somewhere this project does not control. Without a package the
family declares itself not exercised rather than substituting a fixture.

**It never runs by default.** A plain `bash scripts/deploy-probe.sh` skips it and
says so, even with the archive sitting in the repository root. It runs only when
asked for, with `PROBE_ONLY=terminology` or an explicit `FERROEHR_SNOMED_RF2`.

**The setup is: drop the archive in the repository root.** `SnomedCT_*.zip` is
gitignored precisely so it can live there, and the probe finds it, with no
configuration:

```bash
PROBE_ONLY=terminology bash scripts/deploy-probe.sh
```

With both a national and the International edition present the NATIONAL one is
used, because that is the deployment reality being probed. `FERROEHR_SNOMED_RF2`
still overrides if the archive lives elsewhere.

Setting the published checksum is worth the extra line:

```bash
FERROEHR_SNOMED_RF2_MD5=876355868299a8c4d1534e53de6e75a5 \
PROBE_ONLY=terminology bash scripts/deploy-probe.sh
```

The MD5 is the one SNOMED International publishes beside the release: a probe
that silently ran against a truncated or substituted archive would report
conformance about content nobody chose. It is pinned per release, so moving
edition is a deliberate edit.

The family's probes:

| Probe | What it shows |
|---|---|
| `P-TERM-UP` | the overlay's digest-pinned FerroTERM starts and answers its R4B capability statement |
| `P-TERM-IMPORT` | `ferroterm-build` builds the index from the mounted archive (the run prints the wall clock), and the server serves `http://snomed.info/sct` once it opens it |
| `P-TERM-SUBSUMES` | `73211009` subsumes `44054006` over the published hierarchy; the concepts are looked up first, and their absence is a declared gap, never a defect |
| `P-TERM-VALIDATE` | the CDR's `/terminology/*` route answers a SNOMED CT code through FerroTERM, since the bundled openEHR terminology carries none |
| `P-TERM-BROKEN` | with FerroTERM stopped and `fail_on_error` set, a commit of a bound value is refused with `422` |

`ferroterm-build` builds one edition per run. Whether a national Managed Service
edition is complete on its own is answered by what the build reports and what
`P-TERM-SUBSUMES` finds: when the International concepts are absent, the run
says so.

**Resource facts.** FerroTERM's own benchmark records the Netherlands edition at
about 0.9 GB resident once served, and the build unpacks the archive to a 4 GB
tmpfs that dies with the build container. The index stays in the probe
project's named volume on this machine, and a second run rebuilds over it.

