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
#   viewer_launch_key.pem    the viewer's EHR client key at FerroSMART (RS384)
#   viewer.jwks.json         its public half, the JWK Set FerroSMART registers
#
# FerroSMART runs as uid 65532 and the viewer as another non-root user, so
# the files are readable by every user: a local example, never a deployment.
# Needs openssl, xxd and jq. No specification governs this script: our own
# design.
set -euo pipefail

dir="${1:-./secrets/ferrosmart}"
mkdir -p "$dir"

keep() { [[ -s "$dir/$1" ]]; }

keep postgres_password || openssl rand -hex 24 | tr -d '\n' > "$dir/postgres_password"
keep database_url || printf 'postgresql://ferrosmart:%s@ferrosmart-postgres:5432/ferrosmart?sslmode=disable' \
  "$(cat "$dir/postgres_password")" > "$dir/database_url"
for key in signing_key upstream_assertion_key viewer_launch_key; do
  keep "$key.pem" || openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 \
    -out "$dir/$key.pem" 2> /dev/null
done
if ! keep viewer.jwks.json; then
  modulus="$(openssl rsa -in "$dir/viewer_launch_key.pem" -noout -modulus | cut -d= -f2 | xxd -r -p |
    openssl base64 -A | tr '+/' '-_' | tr -d '=')"
  jq -n --arg n "$modulus" '{keys: [{kty: "RSA", kid: "ferroehr-viewer", alg: "RS384", use: "sig", n: $n, e: "AQAB"}]}' \
    > "$dir/viewer.jwks.json"
fi
# Only the files: a directory a caller keeps beside them must stay traversable.
find "$dir" -maxdepth 1 -type f -exec chmod 0644 {} +
echo "the FerroSMART secrets are in $dir"
