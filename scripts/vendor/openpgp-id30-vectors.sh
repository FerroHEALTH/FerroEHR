#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the ML-DSA-65+Ed25519 (RFC 9980 algorithm ID 30) signature vectors
# into corpus/openpgp-id30/: cleartext-signed licence documents, each followed
# by the certificate of the software test key that signed it, one case each;
# the certificates of those test keys trusted as anchors; and the RFC 9980
# Appendix A.3 certificate (A.3.2) and detached signature (A.3.4), the oracle.
#
# The vectors are shared with another verifier of the same signatures, which
# must give the same answer for each, so this script downloads nothing: the
# owner places the files in a directory, and the script finds each one by the
# sha256 it pins and copies it verbatim under its name here.
#
# Usage:
#   scripts/vendor/openpgp-id30-vectors.sh DIR
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

[[ $# -eq 1 && -d "$1" ]] || die "usage: $0 DIR, the directory holding the vector files"
SUPPLIED="$(cd "$1" && pwd)"
DEST="$ROOT/corpus/openpgp-id30"

# `file sha256` per vector, by its name here.
SIGNED=(
  "signed/ok-sha3-256.asc a441076e2a7a0d40a3e27777decf4ceb3af1743feeed2b782b32e9f9c682ad4b"
  "signed/ok-sha512.asc 81e524625ebeca814e02740d5553bd28bd6bb8fcb213c36b083bb1bb38ba867f"
  "signed/weak-digest-sha224.asc f2a90391bac4e1f0a4d1194b187d5bc20c78de9d9a0ae0adcf118f91ce75ce56"
  "signed/tampered.asc 552d28dc031b1a397f7610d40d61a177d9495eb9b418ea6ef8c5e9429c74b469"
  "signed/ed25519-half-broken.asc 3c74a72df8c08dc84570b09405240b985d7763cc41dec0a3ec3b85168c4cbe9f"
  "signed/mldsa-half-broken.asc a059faae5a08f7f2322279089cde2522d1663338f0a755b571c9c87b387251a6"
  "signed/binary-signature.asc 035ab0bc64c322e5e87945941bb32026351b89414c9242b110bf90cef3b324cc"
  "signed/expired-at-signing.asc c2e89bf582dfd01ce78c8e8c132518d68591f0f9ad2fae9b1788ea2811a4a8e1"
  "signed/key-id-only.asc d41b89d043f229c9e4a58113652996065962f582f728e6610444a7a0b56cf3c1"
  "signed/no-certificate.asc 5bd3331df324f113095dc4e7518c5d37994d2d0cbb556c7880e8c2febea383b7"
  "signed/no-direct-key.asc 6f4e8e030deaa557fcaff716c35ec41e4879def5c8db3d2406d55ca66791256c"
  "signed/not-signing.asc 722c79ee1a81578e3ce4d79dd5ea77db92e8f914d104abef661117aaad0d3980"
  "signed/two-signatures.asc afc21873154899efd2fc6766b7a8871ede11a6a8f1da7daf9efa762426e5e855"
  "signed/unknown-field.asc 619d5b58a5cf1ec3227b96324bbaafbba7f540be1f46f8393dc5ae4b6069e13b"
  "signed/unknown-signer.asc 76b76a1fb63681006511b83f27fef3065125584f7093c718586afe8bd9560b82"
  "signed/untrusted.asc cc1e1876351b5988e892e330fc39e562a5a6f8d451f40ab0382e7973ad222cfb"
)
ANCHORS=(
  "anchors/A.asc 9ad7f895591e696377e5b1e662e990303f73c3d076980f48f3c6fae43d187177"
  "anchors/B.asc 4be7ea7e72ca9ade5c3992aecc89c9b482e180bac575e7b54f8261cb2a1b3b2f"
  "anchors/C.asc 256f721989b1c4bd40b1291751420d412b44d343404f9e5fe9b0cee2cc34ba34"
  "anchors/D.asc c06486541bb87f71e0991d5f8853fbb7492ec051764a0f61c8be84689b2382e1"
  "anchors/N.asc 014dc7737b5ce9c7b4262364111fbd83df8ee4d820f6413da9ec7f9611eb6d0c"
)
RFC9980=(
  "rfc9980/a3.2-certificate.asc 4880dead81e043038df20e961ab74a7c61f14245371d6a229c1888be79813764"
  "rfc9980/a3.4-signature.asc af8689111ee427d9f78e5bca17d1d2227ae8787071adf8b1ded66e36a6058c0c"
)

pinned_require shasum
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

for pin in "${SIGNED[@]}" "${ANCHORS[@]}" "${RFC9980[@]}"; do
  pinned_supplied "$SUPPLIED" "$work/tree/${pin% *}" "${pin##* }"
  say "${pin% *}"
done

cat >"$work/tree/PROVENANCE.md" <<EOF
<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the ML-DSA-65+Ed25519 signature vectors

- \`signed/\`: cleartext-signed licence documents (RFC 9580 §7), each a
  version 6 signature by an ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30)
  followed by the certificate of that key (RFC 9580 §10.1), except
  \`no-certificate.asc\`. The keys are software test keys made for these
  vectors and sign nothing else. Each file states one case, and every
  verifier of these documents gives the same answer for it; the licence
  check's tests list each case with its anchors, its day and its answer.
  The signature-level cases:
  - \`ok-sha3-256.asc\` and \`ok-sha512.asc\`: good signatures over SHA3-256
    and SHA2-512; they verify.
  - \`weak-digest-sha224.asc\`: a signature over SHA2-224, refused for its
    digest before any cryptography (RFC 9980 §5.3.1).
  - \`tampered.asc\`: the signed text changed after signing; it does not
    verify.
  - \`ed25519-half-broken.asc\` and \`mldsa-half-broken.asc\`: one half of
    the composite signature changed; it does not verify (RFC 9980 §3.2).
  The licence-level cases: a binary signature, a key that expired before it
  signed, an issuer named by key id only, no certificate, a key with no
  direct-key self-signature, a key not certified for signing, two
  signatures, an unknown licence field, a signature naming another issuer,
  and a key no anchor trusts.
- \`anchors/\`: the certificates of the test keys, trusted as licence
  anchors by the cases.
- \`rfc9980/\`: RFC 9980 Appendix A.3.2 (the transferable public key) and
  A.3.4 (the detached signature over "Testing\\n", a version 6 text
  signature over SHA2-256, hashed as "Testing\\r\\n"), extracted verbatim.
  RFC 9980 is subject to BCP 78; the IETF Trust Legal Provisions
  (<https://trustee.ietf.org/license-info>) allow code components such as
  these to be copied unmodified.
- Licence: \`signed/\` and \`anchors/\` are Cadasto B.V.'s, under BUSL-1.1
  (\`REUSE.toml\`).
- Fetched by: \`scripts/vendor/openpgp-id30-vectors.sh DIR\`, which copies
  each file from the supplied directory by its sha256; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md).

| File | sha256 |
|---|---|
$(pinned_rows "${SIGNED[@]}" "${ANCHORS[@]}" "${RFC9980[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "$((${#SIGNED[@]} + ${#ANCHORS[@]} + ${#RFC9980[@]})) vectors into corpus/openpgp-id30/"
