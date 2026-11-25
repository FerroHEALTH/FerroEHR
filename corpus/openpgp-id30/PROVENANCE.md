<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the ML-DSA-65+Ed25519 signature vectors

- `signed/`: cleartext-signed documents (RFC 9580 §7), each a version 6
  text signature by an ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30)
  followed by the certificate of that key (RFC 9580 §10.1). The keys are
  software test keys made for these vectors and sign nothing else. Each file
  states one case, and every verifier of these signatures gives the same
  answer for it:
  - `ok-sha3-256.asc` and `ok-sha512.asc`: good signatures over SHA3-256
    and SHA2-512; they verify.
  - `weak-digest-sha224.asc`: a signature over SHA2-224, refused for its
    digest before any cryptography (RFC 9980 §5.3.1).
  - `tampered.asc`: the signed text changed after signing; it does not
    verify.
  - `ed25519-half-broken.asc` and `mldsa-half-broken.asc`: one half of
    the composite signature changed; it does not verify (RFC 9980 §3.2).
- `rfc9980/`: RFC 9980 Appendix A.3.2 (the transferable public key) and
  A.3.4 (the detached signature over "Testing\n", a version 6 text
  signature over SHA2-256, hashed as "Testing\r\n"), extracted verbatim.
  RFC 9980 is subject to BCP 78; the IETF Trust Legal Provisions
  (<https://trustee.ietf.org/license-info>) allow code components such as
  these to be copied unmodified.
- Licence: `signed/` is Cadasto B.V.'s, under BUSL-1.1 (`REUSE.toml`).
- Fetched by: `scripts/vendor/openpgp-id30-vectors.sh DIR`, which copies
  each file from the supplied directory by its sha256; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md).

| File | sha256 |
|---|---|
| `signed/ok-sha3-256.asc` | `a441076e2a7a0d40a3e27777decf4ceb3af1743feeed2b782b32e9f9c682ad4b` |
| `signed/ok-sha512.asc` | `81e524625ebeca814e02740d5553bd28bd6bb8fcb213c36b083bb1bb38ba867f` |
| `signed/weak-digest-sha224.asc` | `f2a90391bac4e1f0a4d1194b187d5bc20c78de9d9a0ae0adcf118f91ce75ce56` |
| `signed/tampered.asc` | `552d28dc031b1a397f7610d40d61a177d9495eb9b418ea6ef8c5e9429c74b469` |
| `signed/ed25519-half-broken.asc` | `3c74a72df8c08dc84570b09405240b985d7763cc41dec0a3ec3b85168c4cbe9f` |
| `signed/mldsa-half-broken.asc` | `a059faae5a08f7f2322279089cde2522d1663338f0a755b571c9c87b387251a6` |
| `rfc9980/a3.2-certificate.asc` | `4880dead81e043038df20e961ab74a7c61f14245371d6a229c1888be79813764` |
| `rfc9980/a3.4-signature.asc` | `af8689111ee427d9f78e5bca17d1d2227ae8787071adf8b1ded66e36a6058c0c` |
