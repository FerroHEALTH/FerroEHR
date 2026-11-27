#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Makes the secret files of the FerroSMART overlays (docker-compose.ferrosmart.yml
# and an upstream overlay beside it) into DIR, default ./secrets/ferrosmart. A
# file that exists is kept, so a second run changes nothing.
#
#   postgres_password        FerroSMART's own PostgreSQL
#   database_url             the URL FerroSMART reaches it by
#   signing_key.pem          the RS256 key FerroSMART signs every access token with
#   upstream_assertion_key.pem
#                            the key FerroSMART authenticates to Keycloak with
#                            (private_key_jwt, OpenID Connect Core 1.0 §9)
#   backend_key.pem          the P-384 key of FerroSMART's own Backend Services
#                            client (ES384)
#   viewer_launch_key.pem    the viewer's EHR client key at FerroSMART, ML-DSA-65
#                            (RFC 9964 §5, in the PKCS#8 form of RFC 9881 §6)
#   viewer_client.json       the viewer's client document with the public half
#                            of that key, which `ferrosmart client apply` registers
#
# FerroSMART runs as uid 65532 and the viewer as another non-root user, so
# the files are readable by every user: a local example, never a deployment.
# Needs openssl and jq. The ML-DSA key needs OpenSSL 3.5 or later; with
# an older one, openssl runs from the pinned PostgreSQL image of the overlay,
# so docker is needed too. No specification governs this script: our own
# design.
set -euo pipefail

dir="${1:-./secrets/ferrosmart}"
mkdir -p "$dir"

keep() { [[ -s "$dir/$1" ]]; }

# The overlay's PostgreSQL image (docker-compose.ferrosmart.yml), a Debian
# trixie image whose OpenSSL makes ML-DSA keys.
OPENSSL_IMAGE="postgres:19.0@sha256:aaa10f7ba7328b4a221a6c3184d83eef885bd16bf1f6de864757863156b0a4d1"

# Runs openssl with ML-DSA: the local one when it has ML-DSA-65, else the
# image's, reading stdin and writing stdout.
openssl_ml_dsa() {
  local listed
  listed="$(openssl list -signature-algorithms 2> /dev/null || true)"
  if grep -q 'ML-DSA-65' <<< "$listed"; then
    openssl "$@"
  else
    docker run --rm -i --entrypoint openssl "$OPENSSL_IMAGE" "$@"
  fi
}

keep postgres_password || openssl rand -hex 24 | tr -d '\n' > "$dir/postgres_password"
keep database_url || printf 'postgresql://ferrosmart:%s@ferrosmart-postgres:5432/ferrosmart?sslmode=disable' \
  "$(cat "$dir/postgres_password")" > "$dir/database_url"
for key in signing_key upstream_assertion_key; do
  keep "$key.pem" || openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 \
    -out "$dir/$key.pem" 2> /dev/null
done
keep backend_key.pem || openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-384 \
  -out "$dir/backend_key.pem" 2> /dev/null
if ! keep viewer_launch_key.pem; then
  openssl_ml_dsa genpkey -algorithm ML-DSA-65 > "$dir/viewer_launch_key.pem"
fi
if ! keep viewer_client.json; then
  # RFC 9881 §4: the public key is the last 1952 octets of the ML-DSA-65
  # SubjectPublicKeyInfo, the `pub` of an AKP key (RFC 9964 §3).
  public="$(openssl_ml_dsa pkey -pubout -outform DER < "$dir/viewer_launch_key.pem" | tail -c 1952 |
    openssl base64 -A | tr '+/' '-_' | tr -d '=')"
  jq -n --arg pub "$public" '{
    client_id: "ferroehr-viewer",
    client_name: "FerroEHR viewer",
    token_endpoint_auth_method: "private_key_jwt",
    jwks: {keys: [{kty: "AKP", kid: "ferroehr-viewer", alg: "ML-DSA-65", use: "sig", pub: $pub}]},
    scope: "system/launch.c",
    grant_types: ["client_credentials"],
    resources: ["http://localhost:8080/ferroehr/rest/openehr/v1"]
  }' > "$dir/viewer_client.json"
fi
# Only the files: a directory a caller keeps beside them must stay traversable.
find "$dir" -maxdepth 1 -type f -exec chmod 0644 {} +
echo "the FerroSMART secrets are in $dir"
