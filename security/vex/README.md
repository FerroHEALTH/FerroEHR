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
| `ferroehr-os.openvex.json` | The Debian packages the `ferroehr` and `ferroehr-viewer` images inherit from `gcr.io/distroless/cc-debian13:nonroot` (glibc, the gcc-14 runtime libraries, zlib), one statement per CVE naming both images. Each argument rests on what the one program in each image loads and imports: `libgcc_s`, `libm` and `libc` only, no `dlopen`, no C++. CVE-2026-8674 (the stub resolver's long search domain) is `affected`, because both programs resolve names through `getaddrinfo`; its action statement says how an operator keeps the resolver's search list trusted. | by hand |
| `postgres-os.openvex.json` | The Debian packages of the `ferroehr-postgres` image, one statement per finding, each naming the Debian packages it covers as subcomponents. Six are `affected` with an action statement: three libxml2 findings and two glibc JISX0213 converter findings, which any database session reaches through PostgreSQL's XML functions, and CVE-2026-8674, which the server reaches when an operator configures a host name. | by hand |
| `rust-advisories.openvex.json` | The Rust dependency advisories: the five accepted by the advisory gate, plus the one a lock-file-reading scanner reports for a crate our feature set never compiles. Each statement additionally carries a `ferroehr:reachability` block — our own extension, since OpenVEX defines none — naming the crates the affected package is reached through. | **generated** |

## The hand-written OS documents

The two OS documents argue from the bytes in the image, so they go stale when
the image changes. `ferroehr-os.openvex.json` rests on the linked libraries
and imported symbols of the two Rust programs; re-checking those against
every new binary is tracked in #3694. `postgres-os.openvex.json` rests on what
the PostgreSQL 18 server, the upstream entrypoint and the `gosu` wrapper load
and run, and on the configuration FerroEHR ships (`pg_hba.conf`, no XML in the
schemas, no LDAP, Kerberos or PAM authentication). On every base bump or
Debian security update, rerun the release scan over a candidate image and
re-read every statement whose package changed; a statement whose finding no
longer fires is deleted.

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
