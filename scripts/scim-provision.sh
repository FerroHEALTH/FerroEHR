#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Provisions the persons a development lane logs in as through FerroEHR's
# SCIM endpoint (RFC 7644 §3.3), so each login is bound to a demographic
# PERSON before the lane's run starts.
#
# It mints an RFC 9068 client credentials access token with the committed
# SCIM test key (docs/conformance/party/scim/README.md: TEST MATERIAL, never
# production) using openssl alone, then creates each User with POST
# /scim/v2/Users. A User whose userName exists already (409 `uniqueness`) is
# taken as provisioned, so the step runs again on a kept volume. No openEHR
# spec governs provisioning a lane: our own design.
#
# Usage:
#   scripts/scim-provision.sh <base-url> <client-id> <user>[=<login-key>]...
#
#   <base-url>   the server origin, such as http://localhost:8080
#   <client-id>  the [[scim.client]] the token names; its binding_issuer
#                issues every key this call provisions
#   <user>       the SCIM userName; <login-key> is the binding key the login
#                carries (loginIdentifier), the userName when omitted
#
# Environment (each defaults to the dev lanes' SCIM issuer):
#   SCIM_KEY_FILE   the RS256 private key (docs/conformance/party/scim/scim-test.key.pem)
#   SCIM_ISSUER     the token's `iss` (https://scim.ferroehr.invalid)
#   SCIM_AUDIENCE   the token's `aud` (ferroehr-scim)
#   SCIM_SCOPE      the client's scope (ferroehr/scim)
#   SCIM_KID        the key id the JWKS names (scim-test)
set -euo pipefail

usage() {
  echo "usage: scim-provision.sh <base-url> <client-id> <user>[=<login-key>]..." >&2
  exit 2
}
[[ $# -ge 3 ]] || usage

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base_url="${1%/}"
client_id="$2"
shift 2
key_file="${SCIM_KEY_FILE:-$root/docs/conformance/party/scim/scim-test.key.pem}"
issuer="${SCIM_ISSUER:-https://scim.ferroehr.invalid}"
audience="${SCIM_AUDIENCE:-ferroehr-scim}"
scope="${SCIM_SCOPE:-ferroehr/scim}"
kid="${SCIM_KID:-scim-test}"
extension="urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User"

# RFC 7515 §2: base64url without padding.
b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

now="$(date +%s)"
header="$(jq -cn --arg kid "$kid" '{alg: "RS256", typ: "at+jwt", kid: $kid}' | b64url)"
payload="$(jq -cn --arg iss "$issuer" --arg aud "$audience" --arg client "$client_id" \
  --arg scope "$scope" --arg jti "$(openssl rand -hex 16)" --argjson now "$now" \
  '{iss: $iss, aud: $aud, sub: $client, client_id: $client, scope: $scope,
    iat: $now, exp: ($now + 600), jti: $jti}' | b64url)"
signature="$(printf '%s.%s' "$header" "$payload" | openssl dgst -sha256 -sign "$key_file" -binary | b64url)"
token="$header.$payload.$signature"

body_file="$(mktemp)"
trap 'rm -f "$body_file"' EXIT

for entry in "$@"; do
  user="${entry%%=*}"
  key="${entry#*=}"
  body="$(jq -cn --arg user "$user" --arg key "$key" --arg ext "$extension" \
    '{schemas: ["urn:ietf:params:scim:schemas:core:2.0:User", $ext],
      userName: $user, ($ext): {loginIdentifier: $key}}')"
  status="$(curl -sS -o "$body_file" -w '%{http_code}' -X POST \
    -H "Authorization: Bearer $token" -H 'Content-Type: application/scim+json' \
    --data "$body" "$base_url/scim/v2/Users")"
  case "$status" in
    201) echo "scim-provision: created $user" ;;
    409)
      # NOTE: RFC 7644 §3.3: a 409 `uniqueness` may name a userName of another
      # client, so the User counts as provisioned only when this client sees it.
      found="$(curl -sS -G -H "Authorization: Bearer $token" \
        --data-urlencode "filter=userName eq \"$user\"" "$base_url/scim/v2/Users" |
        jq --arg ext "$extension" --arg key "$key" \
          '[.Resources[]? | select(.[$ext].loginIdentifier == $key)] | length')"
      if [[ "$found" == 1 ]]; then
        echo "scim-provision: $user is provisioned already"
      else
        echo "scim-provision: $user refused with 409: $(cat "$body_file")" >&2
        exit 1
      fi
      ;;
    *)
      echo "scim-provision: $user refused with $status: $(cat "$body_file")" >&2
      exit 1
      ;;
  esac
done
