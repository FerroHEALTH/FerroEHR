<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Post-quantum inventory

This page lists every cryptographic use in FerroEHR, the family crates it
runs on and its release workflows, the algorithm each uses, and whether it
resists a cryptographically relevant quantum computer. The machine-readable
copy is `conformance/post-quantum.tsv`: after its `#` comment lines, one row
per use with five tab-separated columns: `use`, `algorithm`, `safe` (`yes`
or `no`), `pq_standard` (the standard that defines the post-quantum form, or
`no standard yet`) and `location`. Every change that adds, removes or
changes a cryptographic use updates the file in the same pull request, and
`scripts/render/post-quantum.sh` rewrites the table below and the README
badge from it. CI fails when the table, the badge or a location named in the
file disagrees with the tree.

## The sources

- **NIST IR 8547 (final):** ECDSA, EdDSA and RSA signatures (§4.1.1) and
  ECDH, DH and RSA key establishment (§4.1.2) are deprecated after 2030 and
  disallowed after 2035. Symmetric primitives with at least 128 bits of
  classical security, AES-128 and up, SHA-256 and HMAC-SHA-256 among them,
  stay (§4.1.3). FIPS 203, 204 and 205 define ML-KEM, ML-DSA and SLH-DSA.
- **The EU coordinated implementation roadmap (NIS Cooperation Group,
  v1.1):** a use case is high-risk when its confidentiality must hold for
  10 years or more, which health records need, and high-risk use cases
  complete the transition by 31.12.2030 (§4.2). After 2030 they use no
  quantum-vulnerable public-key mechanism stand-alone, and standardised
  hybrid solutions are recommended (§4.1).
- **RFC 10024:** the TLS 1.3 hybrid group X25519MLKEM768 (codepoint
  0x11EC), within the hybrid design of RFC 9954.
- **RFC 9980:** post-quantum OpenPGP. **RFC 9964:** ML-DSA for JOSE and
  COSE. **RFC 9881:** ML-DSA in X.509.

A row is `safe` when every key it depends on is established or verified by
a post-quantum or hybrid mechanism, or when it is a symmetric primitive at
128 bits or more. A row whose post-quantum form has no standard yet counts
as not safe and stays out of the badge, which counts the safe rows of the
rows a post-quantum standard covers; the `no standard yet` rows of the table
below are the rest.

## What FerroEHR does

The REST and viewer listeners come from `ferrohealth-server` 0.12.4, which
offers X25519MLKEM768 first in the TLS 1.3 key exchange, then X25519, P-256
and P-384 for a peer that lacks the hybrid group. It enables rustls's
`prefer-post-quantum` feature, and the lockfile holds no other TLS library,
so every other rustls user in FerroEHR takes the same order from the default
provider: the clients FerroEHR builds itself (the ITI-20 syslog connection
and the multimedia S3 client), the outbound integration clients built
through `ferrohealth-smart`, the other family clients and the PostgreSQL
connection of `sqlx`. The PostgreSQL connection agrees on the hybrid group
when the server's TLS library offers it.

Tests show the listener agreeing on X25519MLKEM768 with a client that offers
it and on X25519 with a client that offers only that
(`app/ferroehr-rest/tests/it/server_tls.rs`), and the syslog connection, the
outbound integration client and the S3 client agreeing on X25519MLKEM768
with a peer that offers it beside X25519, and on X25519 with a peer that
offers only that (`app/ferroehr/src/system_log/syslog.rs`,
`app/ferroehr/tests/it/post_quantum.rs`). The peer is `testkit::kx::KxPeer`.

The key exchange protects what a session carries against an attacker who
records it now and decrypts it later, the risk the roadmap gives first
place. The certificate signatures that authenticate a peer stay classical.
RFC 9881 (October 2025, Proposed Standard) defines ML-DSA keys and
signatures in X.509 certificates. The TLS 1.3 signature schemes for ML-DSA
(`mldsa44` 0x0904, `mldsa65` 0x0905, `mldsa87` 0x0906) are defined by
draft-ietf-tls-mldsa-06, which the IESG approved and which waits in the RFC
Editor queue, not yet an RFC
(<https://datatracker.ietf.org/doc/draft-ietf-tls-mldsa/>, read
2026-11-27). The rows stay `no` while that document is a draft and the
operator's certificates decide the algorithm.

An outbound connection verifies the peer's chain by the root it ends at
(`ferrohealth-server` 0.14 `PlatformTls`). A chain to a root of
`trust_roots_file` or `trust_roots_without_revocation_file` is verified by
webpki on every operating system, so it can carry ML-DSA certificates. A
chain to a platform root goes to the platform's verifier, and the macOS
verifier refuses ML-DSA (FerroHEALTH/FerroSYS#470), so the inventory scores
the two apart.

## JOSE signatures

`ferrohealth-smart` 0.35.0 signs and verifies every JWS in its `jws` module,
ML-DSA-44, ML-DSA-65 and ML-DSA-87 (RFC 9964, `AKP` keys) among the
algorithms. What it accepts on a verification is the list it is given:

- **Access tokens and DPoP proofs:** an issuer's `algorithms` and
  `[auth.dpop] algorithms` default to RS256, RS384, PS256, PS384, ES256 and
  ES384. An ML-DSA parameter set is accepted where a list names it.
- **The viewer's sign-in ID Token:** the relying party's default is RS256 to
  PS512, ES256, ES384 and EdDSA; the viewer adds the three ML-DSA parameter
  sets to it.
- **Client assertions:** a key signs with the algorithm of its kind, an
  ML-DSA key (RFC 9881 §6) with its parameter set and an RSA key with RS384.

`[auth] post_quantum_only = true` in `ferroehr.toml` narrows every
algorithm list of a JWT issuer and of `[auth.dpop]` left at its default to
the three ML-DSA parameter sets, and stops the start when a list names
another algorithm. The viewer's own `[auth] post_quantum_only = true`
accepts an ID Token signed with ML-DSA alone. Both are off by default, for
the issuers that still sign classically. The viewer signs its SMART launch
client assertion with ML-DSA when its key is an ML-DSA key, as the
FerroSMART overlay's `docker/ferrosmart/secrets.sh` makes it, and the
outbound client credentials grant does so with an ML-DSA key in
`private_key_file`.

The badge counts the JOSE rows in that configuration: both
`post_quantum_only` settings on and ML-DSA keys for the client assertions.
A deployment that leaves the settings off accepts the classical algorithms
the rows name, and those rows are then not post-quantum safe for it.

The OpenPGP signatures FerroEHR makes (`VERSION.signature` in `pgp` mode,
the access-log checkpoints and bundles and the system-log seals) are the
RFC 9980 composite ML-DSA-65+Ed25519 (algorithm ID 30) through
`ferrohealth-openpgp-pq`, and a classical signing key is refused at boot
(#6600). Verification against the current and retired keys stays `no`:
the signatures releases before it stored keep verifying against their
retired classical certificates. Licence token verification accepts
ML-DSA-65+Ed25519 anchors through the same path (#6607) and stays `no` while
the Licensor's anchors are Ed25519, until a hardware token can hold an
ML-DSA key.

## The inventory

| Use | Algorithm | Safe | Post-quantum standard | Location |
|---|---|---|---|---|
| TLS 1.3 key exchange of the REST and viewer listeners | X25519MLKEM768 first; X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-server tls, app/ferroehr-rest/src/lib.rs (serve_full), app/ferroehr-viewer/src/cli.rs` |
| TLS 1.3 key exchange of the clients FerroEHR builds itself: the ITI-20 syslog connection and the multimedia S3 client | X25519MLKEM768 first (rustls prefer-post-quantum); X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `app/ferroehr/src/system_log/syslog.rs (repository), app/ferroehr-ext/src/multimedia/blob.rs (from_params)` |
| TLS 1.3 key exchange of the outbound integration clients: the ITI-20 FHIR feed, the PIXm query, the PMIR subscription, the terminology server and the viewer's CDR and SMART launch clients | X25519MLKEM768 first; X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-smart client outbound, app/ferroehr/src/system_log/feed.rs, app/ferroehr/src/service/ehr_index/ihe.rs, app/ferroehr/src/service/demographic/identity_feed/subscribe.rs, app/ferroehr/src/service/terminology/fhir.rs, app/ferroehr-viewer/src/cdr.rs, app/ferroehr-viewer/src/launch.rs` |
| TLS 1.3 key exchange of the family clients: the issuer key sets, the viewer's OpenID Provider, the OTLP export and the FerroPULSE report | X25519MLKEM768 first (rustls prefer-post-quantum); X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-smart resource, ferrohealth-smart oidc, ferrohealth-telemetry otlp, ferrohealth-usage-report` |
| TLS 1.3 key exchange of the PostgreSQL connection | X25519MLKEM768 first (rustls prefer-post-quantum), agreed when the server's TLS library offers it | yes | RFC 10024 | `Cargo.toml (sqlx tls-rustls-aws-lc-rs)` |
| TLS 1.2 key exchange where min_version lowers the floor | ECDHE over X25519, P-256 or P-384 | no | no standard yet | `ferrohealth-server tls (protocol_versions)` |
| TLS certificate authentication of the listeners and of the client certificates FerroEHR presents | ECDSA, RSA or Ed25519, as the operator's certificates hold | no | RFC 9881; draft-ietf-tls-mldsa-06 in the RFC Editor queue | `ferrohealth-server tls, ferrohealth-server tls_client` |
| Outbound TLS certificate verification, a chain to a root of trust_roots_file or trust_roots_without_revocation_file | ML-DSA (RFC 9881), ECDSA, RSA or Ed25519 certificates, verified by webpki on every operating system, the first with a revocation check | no | RFC 9881; draft-ietf-tls-mldsa-06 in the RFC Editor queue | `ferrohealth-server tls_client (PlatformTls), ferrohealth-smart client outbound, app/ferroehr/src/system_log/syslog.rs, app/ferroehr/src/outbound.rs` |
| Outbound TLS certificate verification, a chain to a platform root | ECDSA, RSA or Ed25519, or ML-DSA where the platform's verifier takes it; the macOS verifier refuses ML-DSA (FerroHEALTH/FerroSYS#470) | no | no standard yet | `ferrohealth-server tls_client (PlatformTls), ferrohealth-smart client outbound, app/ferroehr/src/system_log/syslog.rs` |
| TLS record protection | AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305 | yes | NIST IR 8547 4.1.3 | `ferrohealth-server tls` |
| VERSION.signature in digest mode | SHA-256 of the canonical form | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/versioning/signature/signer.rs` |
| VERSION.signature in pgp mode, signing | OpenPGP v6 detached signature over SHA2-512, ML-DSA-65+Ed25519 (RFC 9980 algorithm ID 30); a classical signing key is refused at boot | yes | RFC 9980 | `app/ferroehr/src/versioning/signature/key.rs` |
| VERSION.signature in pgp mode, verification against the current and retired keys | ML-DSA-65+Ed25519 through ferrohealth-openpgp-pq; the signatures stored before it against the retired Ed25519, RSA, ECDSA or Ed448 certificates | no | RFC 9980 | `app/ferroehr/src/openpgp.rs, app/ferroehr/src/versioning/signature/key.rs` |
| Access-log checkpoint and exported bundle signatures | OpenPGP, the signing keyring's ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30) | yes | RFC 9980 | `app/ferroehr/src/access_log/checkpoint.rs, app/ferroehr/src/access_log/store.rs` |
| Verification of imported access-log bundles and checkpoints | OpenPGP, the algorithm each trusted system's certificate holds: ML-DSA-65+Ed25519 or classical | no | RFC 9980 | `app/ferroehr/src/openpgp.rs, app/ferroehr/src/versioning/signature/trust.rs, app/ferroehr/src/access_log/imported.rs` |
| System-log seal signatures | OpenPGP, the signing keyring's ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30) | yes | RFC 9980 | `app/ferroehr/src/system_log/seal.rs` |
| Access-log and system-log seal chains and bundle digests | SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/access_log/seal.rs, app/ferroehr/src/access_log/chain.rs, app/ferroehr/src/system_log/content_digest.rs` |
| Content digests and content keys: the DV_MULTIMEDIA integrity check and blob key, the access-log facts digest and the digest keys of results and settings | SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/digest.rs, app/ferroehr-ext/src/multimedia/offload.rs, app/ferroehr-ext/src/multimedia/blob.rs, app/ferroehr/src/access_log/stored_form/parts/facts.rs` |
| Licence token verification | OpenPGP cleartext signature by the Licensor's key: the Ed25519 anchors in use; ML-DSA-65+Ed25519 anchors verified the same way | no | RFC 9980 | `app/ferroehr/src/licence/verify.rs, app/ferroehr/src/openpgp.rs, app/ferroehr/src/licence/token.rs` |
| Licence stamp | HMAC-SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/licence/stamp.rs` |
| Basic password hashing | Argon2id | yes | NIST IR 8547 4.1.3 | `ferrohealth-smart basic, app/ferroehr-rest/src/extensions/access/authn` |
| Access token and DPoP proof verification with [auth] post_quantum_only = true | ML-DSA-44, ML-DSA-65 or ML-DSA-87 (AKP) alone; off (the default), RS256, RS384, PS256, PS384, ES256 and ES384, and an ML-DSA parameter set where an algorithms list names it | yes | RFC 9964 | `ferrohealth-smart resource, app/ferroehr/src/config/auth_section.rs, app/ferroehr-rest/src/extensions/access/authn` |
| Client assertions of the outbound client credentials grant (private_key_jwt) with an ML-DSA key | ML-DSA-44, ML-DSA-65 or ML-DSA-87, the parameter set of the key; with a classical key RS384 (RSA) by default, RS256 to PS512, ES256, ES384 or EdDSA | yes | RFC 9964 | `ferrohealth-smart client assertion, app/ferroehr/src/outbound.rs, app/ferroehr/tests/it/outbound_ml_dsa.rs` |
| Viewer SMART launch client assertions with an ML-DSA key, as the FerroSMART overlay makes it | ML-DSA-44, ML-DSA-65 or ML-DSA-87, the parameter set of the key (ML-DSA-65 in the overlay); with a classical key RS384 or ES384 | yes | RFC 9964 | `app/ferroehr-viewer/src/launch.rs, docker/ferrosmart/secrets.sh` |
| Viewer sign-in ID token verification with the viewer's [auth] post_quantum_only = true | ML-DSA-44, ML-DSA-65 or ML-DSA-87 alone; off (the default), those beside RS256, RS384, RS512, PS256, PS384, PS512, ES256, ES384 and EdDSA | yes | RFC 9964 | `ferrohealth-smart oidc, app/ferroehr-viewer/src/config.rs, app/ferroehr-viewer/src/cli.rs` |
| Helm chart signatures | cosign keyless (Sigstore, ECDSA P-256) | no | no standard yet | `.github/workflows/build-chart.yml` |
| crates.io publishing | Trusted Publishing (OIDC token from GitHub, RS256) | no | no standard yet | `.github/workflows/publish-crates.yml` |
| Release, image and chart provenance and SBOM attestations | Sigstore attestation (ECDSA P-256) | no | no standard yet | `.github/workflows/build-image.yml, .github/workflows/release-build.yml, .github/workflows/build-chart.yml` |
