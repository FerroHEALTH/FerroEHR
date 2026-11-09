# VEX statements

Vulnerability Exploitability eXchange documents, in [OpenVEX](https://openvex.dev)
format, asserting the exploitability of specific findings in the images this
project publishes.

A VEX document is not a way to silence a scanner. It is the machine-readable form
of an argument, and it carries the argument with it: each statement names the
vulnerability, the product, a `status`, a controlled-vocabulary `justification`,
and an `impact_statement` a reader can check. The alternative — an ignore list —
records the decision without the reasoning, which decays into a list nobody can
re-evaluate.

## Rules

- **`not_affected` needs a justification from the OpenVEX vocabulary**, and the
  `impact_statement` must say concretely why the code is unreachable in *our*
  usage. "Low risk" is not a justification.
- **A finding we can fix is fixed, not VEXed.** These documents exist for
  findings in inherited upstream layers, where the fix belongs to someone else.
- **Re-check on every base-image bump.** When an upstream image rebuilds its
  bundled binaries, statements about them become obsolete and the entries go —
  a stale `not_affected` is worse than no VEX at all.
- The scanners consume these files (`trivy --vex`), so a statement that stops
  being true stops being invisible: the finding returns and the gate fails.
- **An `affected` statement leaves its finding visible.** `trivy --vex`
  removes a finding only for `not_affected` or `fixed`; an `affected` finding
  stays in every scan with its action statement beside it in the release
  record, which is intended: it applies to the reader until the fix ships.
- **Every finding a release ships carries a statement.** The release pipeline
  scans each image and server binary with no severity floor and unfixed
  findings included, and `scripts/checks/vex-coverage.sh` refuses the release
  while a finding has no statement here that judges it: `not_affected` with a
  `justification` or `impact_statement`, `affected` with an `action_statement`,
  or `fixed`. Products are named `pkg:oci/<image>` and
  `pkg:cargo/ferroehr-server`, and a statement matches a finding by its
  `vulnerability.name` or an `aliases` entry.

## Documents

| File | Subject | Authored |
|---|---|---|
| `ferroehr-os.openvex.json` | The Debian packages the `ferroehr` and `ferroehr-viewer` images inherit from `gcr.io/distroless/cc-debian13:nonroot` (glibc and the gcc-14 runtime libraries), one statement per CVE naming both images. Each argument rests on what the one program in each image loads and imports: `libgcc_s`, `libm` and `libc` only, no `dlopen`, no C++. CVE-2026-8674 (the stub resolver's long search domain) is `affected`, because both programs resolve names through `getaddrinfo`; its action statement says how an operator keeps the resolver's search list trusted. | by hand |
| `postgres-os.openvex.json` | The Debian packages of the `ferroehr-postgres` image, one statement per finding, each naming the Debian packages it covers as subcomponents. Ten are `affected` with an action statement: three libxml2 findings and two glibc JISX0213 converter findings, which any database session reaches through PostgreSQL's XML functions; CVE-2026-8674, which the server reaches when an operator configures a host name; CVE-2026-107778, which a `gss` entry in `pg_hba.conf` reaches; and three zlib findings (CVE-2026-27171, CVE-2026-76844, CVE-2026-85091) in zlib1g `1:1.3.dfsg+really1.3.1-1+b1`, which the image of 5.6.3 and earlier releases carries and whose action is the upgrade to 5.7.0, since Debian fixes them in trixie-security and the image's upgrade layer installs the fix. | by hand |
| `rust-advisories.openvex.json` | The Rust dependency advisories: the three accepted by the advisory gate, and any a lock-file-reading scanner reports for a crate our feature set never compiles (none today). Each statement additionally carries a `ferroehr:reachability` block — our own extension, since OpenVEX defines none — naming the crates the affected package is reached through. | **generated** |
| `rust-releases.openvex.json` | A Rust dependency vulnerability that a published release carries and a later release fixes, judged per release line so the published-image scan of the earlier release has a judgement while the finding stays visible. CVE-2026-115905 in `zip` 9.0.4 is `affected` in the server binary and the `ferroehr` image of 5.5.0 and earlier; its action statement is the upgrade to 5.6.0, which locks `zip` 9.1.3. | by hand |

## The hand-written OS documents

The two OS documents argue from the bytes in the image, so they go stale when
the image changes. `ferroehr-os.openvex.json` rests on the linked libraries
and imported symbols of the two Rust programs, and each statement that does
carries those facts as data in a `ferroehr:linkage` block (our own extension,
like `ferroehr:reachability`): the exact DT_NEEDED set with the loader per
architecture, `rpath: false`, the imports it rules out by name
(`forbidden_imports`) or prefix (`forbidden_import_prefixes`, `_Z` for any C++
symbol), the symbol versions it rules out (`forbidden_version_prefixes`), and
byte strings the file must not contain (`forbidden_strings`).
`scripts/checks/vex-linkage.sh` reads those facts off each binary
with `readelf` in the release lane, for both programs on both architectures,
after the strip and before the image is built, and refuses the release when a
binary contradicts a statement; its `--self-test` drives the detector over
synthetic `readelf` listings and over every import the document rules out. A
statement whose prose changes changes its block in the same edit, and the
document's `version` and `timestamp` move with it. `postgres-os.openvex.json` rests on what
the PostgreSQL 18 server, the upstream entrypoint and the `gosu` wrapper load
and run, and on the configuration FerroEHR ships (`pg_hba.conf`, no XML in the
schemas, no LDAP, Kerberos or PAM authentication). On every base bump or
Debian security update, rerun the release scan over a candidate image and
re-read every statement whose package changed; a statement whose finding no
longer fires is deleted.

## A judgement trivy does not apply

The gates hand these documents to `trivy --vex`, so a statement takes effect
only when trivy applies it. Trivy 0.73.0 to 0.76.0 keeps a finding whose
package has a dependency cycle among its dependents (#5158): its walk from the
package to the image treats a component whose dependents are all on the
current path like one with no dependent at all, and fails safe to "affected".
The `ferroehr-postgres` image carries a full dpkg database, whose `Depends`
form two such cycles (`libc6` with `libgcc-s1`, `libssl3t64` with
`openssl-provider-legacy`). So no `not_affected` statement for `libc6`,
`libgcc-s1`, `gcc-14-base` or `zlib1g` in `postgres-os.openvex.json` changes
what the gate reports, while the statements for every other package apply.
The distroless images have no dependency graph, so every statement in
`ferroehr-os.openvex.json` applies. This is why the scheduled scan raised the
`zlib1g` findings in #5137.

`scripts/security/vex-applied.sh --image <name>=<ref>` scans an image for both
platforms with and without the documents, under the same flags, and names
every `not_affected` or `fixed` judgement the second scan still reports. Run
it after adding a judgement for the postgres image. A judgement it names
still counts for the release record (`vex-coverage.sh` reads the documents
itself), but the gate keeps the finding: a fixable one is fixed by the
rebuild, as for any other. On 2026-11-08, over the published images, it
applied 237 of 266 judged findings of `ferroehr-postgres` per platform (the
29 left are the `libc6`, `libgcc-s1` and `gcc-14-base` ones) and 44 of 44 of
`ferroehr`.

## The generated document

`rust-advisories.openvex.json` is produced by
`scripts/security/vex-generate.sh` from two inputs, and must never be edited by
hand:

- **`deny.toml`** `[advisories].ignore` — the authoritative set of advisory
  ids. It is the gate that actually decides whether a build passes, so it is
  the only place the id list may live.
- **`security/vex/rust-advisories.toml`** — the reasoning: the OpenVEX
  `status`, the controlled-vocabulary `justification`, and the
  `impact_statement` for each id.

Two lists that must agree is exactly the shape this repository has already been
bitten by (a second advisory ignore list at `.cargo/audit.toml` that nothing
read and that had drifted to a different set of ids). So the generator refuses
to emit anything unless the two sets match in **both** directions, and
`scripts/checks/vex-advisories.sh` — the `vex` CI job — regenerates the
document and fails on any difference. Adding an ignore to `deny.toml` without
publishing its justification is a red build, not an oversight nobody notices.

Agreement is not the same as truth, though: an ignore and its justification stay
in perfect agreement while a dependency upgrade quietly resolves the advisory
underneath both. `scripts/checks/advisory-exceptions.sh` (in the `cargo-deny` CI
job, where the dependency graph is resolvable) closes that half — it promotes
cargo-deny's `advisory-not-detected` diagnostic to an error, so an exception that
has outlived its finding fails the build instead of ageing into a false claim.

And a statement can be in agreement, describe an advisory that still fires, and
still argue from a dependency path that is not the real one. That happened here:
the `rsa` statement asserted the crate was reached only through `openidconnect`,
where RSA verifies with a public key, while `cargo tree -i rsa` had shown a
second path through `pgp`, where an RSA operation would be a private-key one —
which is what the advisory is about. So each statement carries its path as data
(`carriers`, `workspace_roots`) and `scripts/checks/vex-reachability.sh` — also
in the `cargo-deny` job — compares both sets against the graph cargo resolves,
exhaustively in both directions. They travel into the published document as a
`ferroehr:reachability` block on each statement, so a consumer reads the same
claim the gate checks.

To change a statement: edit `rust-advisories.toml` (and `deny.toml` if the id
set changes), bump the document's `version` and `timestamp`, then run
`bash scripts/security/vex-generate.sh`. If the change is about where a crate
enters the build, `carriers` and `workspace_roots` come from
`cargo tree -i <crate>[@<version>] --workspace --all-features --target all -e
normal,build,dev` — the gate will not accept a set you have not read off the
graph.

### Why lock-file-only findings are in there

The last section of `rust-advisories.toml` carries advisories `cargo-deny`
never raises — because it resolves cargo FEATURES — but which a scanner reading
`Cargo.lock` alone does report. Those are precisely the findings that reach a
downstream consumer with no explanation attached anywhere in this repository,
which is the reason a published VEX document is worth more here than a comment.
They are deliberately absent from `deny.toml`'s ignore list: an ignore for an
advisory the gate never raises records nothing and would start applying
silently if the tool ever became lock-file-based. The generator enforces that
too.
