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
as not safe, and the badge counts the safe rows of all rows.

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
place. The certificate signatures that authenticate a peer stay classical:
no TLS 1.3 signature scheme for ML-DSA is published as an RFC yet, and the
operator's certificates decide the algorithm. The JOSE signatures of the
access tokens, the DPoP proofs, the client assertions and the viewer's
sign-in stay classical until the issuers FerroEHR works with sign with
ML-DSA (RFC 9964).

The OpenPGP signatures (`VERSION.signature` in `pgp` mode, the access-log
checkpoints and bundles and the system-log seals) are Ed25519 or RSA on
`main`. Pull request #6614 moves them to the RFC 9980 composite
ML-DSA-65+Ed25519 (algorithm ID 30) through `ferrohealth-openpgp-pq`, with a
classical key only behind `record.signing.allow_classical_key`; when it
merges, the signing, checkpoint and seal rows turn `yes` and the
verification row stays `no` while classical retired keys verify the
signatures already stored. Licence token verification moves to ID 30 with
#6607.

## The inventory

| Use | Algorithm | Safe | Post-quantum standard | Location |
|---|---|---|---|---|
| TLS 1.3 key exchange of the REST and viewer listeners | X25519MLKEM768 first; X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-server tls, app/ferroehr-rest/src/lib.rs (serve_full), app/ferroehr-viewer/src/cli.rs` |
| TLS 1.3 key exchange of the clients FerroEHR builds itself: the ITI-20 syslog connection and the multimedia S3 client | X25519MLKEM768 first (rustls prefer-post-quantum); X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `app/ferroehr/src/system_log/syslog.rs (repository), app/ferroehr-ext/src/multimedia/blob.rs (from_params)` |
| TLS 1.3 key exchange of the outbound integration clients: the ITI-20 FHIR feed, the PIXm query, the PMIR subscription, the terminology server and the viewer's CDR and SMART launch clients | X25519MLKEM768 first; X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-smart client outbound, app/ferroehr/src/system_log/feed.rs, app/ferroehr/src/service/ehr_index/ihe.rs, app/ferroehr/src/service/demographic/identity_feed/subscribe.rs, app/ferroehr/src/service/terminology/fhir.rs, app/ferroehr-viewer/src/cdr.rs, app/ferroehr-viewer/src/launch.rs` |
| TLS 1.3 key exchange of the family clients: the issuer key sets, the viewer's OpenID Provider, the OTLP export and the FerroPULSE report | X25519MLKEM768 first (rustls prefer-post-quantum); X25519, P-256, P-384 for a peer without it | yes | RFC 10024 | `ferrohealth-smart resource, ferrohealth-smart oidc, ferrohealth-telemetry otlp, ferrohealth-usage-report` |
| TLS 1.3 key exchange of the PostgreSQL connection | X25519MLKEM768 first (rustls prefer-post-quantum), agreed when the server's TLS library offers it | yes | RFC 10024 | `Cargo.toml (sqlx tls-rustls-aws-lc-rs)` |
| TLS 1.2 key exchange where min_version lowers the floor | ECDHE over X25519, P-256 or P-384 | no | no standard yet | `ferrohealth-server tls (protocol_versions)` |
| TLS server and client certificate authentication | ECDSA, RSA or Ed25519, as the operator's certificates hold | no | no standard yet | `ferrohealth-server tls, ferrohealth-server tls_client` |
| TLS record protection | AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305 | yes | NIST IR 8547 4.1.3 | `ferrohealth-server tls` |
| VERSION.signature in digest mode | SHA-256 of the canonical form | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/versioning/signature/signer.rs` |
| VERSION.signature in pgp mode, signing | OpenPGP detached signature over SHA-256, Ed25519 (v4 EdDSA or RFC 9580) or RSA | no | RFC 9980 | `app/ferroehr/src/versioning/signature/key.rs` |
| VERSION.signature in pgp mode, verification against the current and retired keys | OpenPGP, the algorithm each configured key holds | no | RFC 9980 | `app/ferroehr/src/versioning/signature/verify.rs, app/ferroehr/src/versioning/signature/signer.rs` |
| Access-log checkpoint and exported bundle signatures | OpenPGP, the signing keyring's Ed25519 or RSA key | no | RFC 9980 | `app/ferroehr/src/access_log/checkpoint.rs, app/ferroehr/src/access_log/store.rs` |
| Verification of imported access-log bundles and checkpoints | OpenPGP, the algorithm each trusted system's certificate holds | no | RFC 9980 | `app/ferroehr/src/versioning/signature/trust.rs, app/ferroehr/src/access_log/imported.rs` |
| System-log seal signatures | OpenPGP, the signing keyring's Ed25519 or RSA key | no | RFC 9980 | `app/ferroehr/src/system_log/seal.rs` |
| Access-log and system-log seal chains and bundle digests | SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/access_log/seal.rs, app/ferroehr/src/access_log/chain.rs, app/ferroehr/src/system_log/content_digest.rs` |
| Content digests and content keys: the DV_MULTIMEDIA integrity check and blob key, the access-log facts digest and the digest keys of results and settings | SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/digest.rs, app/ferroehr-ext/src/multimedia/offload.rs, app/ferroehr-ext/src/multimedia/blob.rs, app/ferroehr/src/access_log/stored_form/parts/facts.rs` |
| Licence token verification | OpenPGP cleartext signature by the Licensor's Ed25519 key | no | RFC 9980 | `app/ferroehr/src/licence/verify.rs, app/ferroehr/src/licence/token.rs` |
| Licence stamp | HMAC-SHA-256 | yes | NIST IR 8547 4.1.3 | `app/ferroehr/src/licence/stamp.rs` |
| Basic password hashing | Argon2id | yes | NIST IR 8547 4.1.3 | `ferrohealth-smart basic, app/ferroehr-rest/src/extensions/access/authn` |
| Access token and DPoP proof verification | RS256, RS384, PS256, PS384, ES256, ES384 | no | RFC 9964 | `ferrohealth-smart resource, app/ferroehr-rest/src/extensions/access/authn` |
| Client assertions of the outbound client credentials grant (private_key_jwt) | RS256, RS384, RS512, PS256, PS384, PS512, ES256, ES384 or EdDSA, as the key holds | no | RFC 9964 | `ferrohealth-smart client assertion, app/ferroehr/src/outbound.rs` |
| Viewer SMART launch client assertions | RS384 or ES384 | no | RFC 9964 | `app/ferroehr-viewer/src/launch.rs` |
| Viewer sign-in ID token verification | RS256, RS384, RS512, PS256, PS384, PS512, ES256, ES384, EdDSA | no | RFC 9964 | `ferrohealth-smart oidc, app/ferroehr-viewer/src/cli.rs` |
| Viewer session cookie in sealed mode | AES-256-GCM | yes | NIST IR 8547 4.1.3 | `ferrohealth-session sessions, app/ferroehr-viewer/src/config.rs` |
| Helm chart signatures | cosign keyless (Sigstore, ECDSA P-256) | no | no standard yet | `.github/workflows/build-chart.yml` |
| crates.io publishing | Trusted Publishing (OIDC token from GitHub, RS256) | no | no standard yet | `.github/workflows/publish-crates.yml` |
| Release, image and chart provenance and SBOM attestations | Sigstore attestation (ECDSA P-256) | no | no standard yet | `.github/workflows/build-image.yml, .github/workflows/release-build.yml, .github/workflows/build-chart.yml` |
