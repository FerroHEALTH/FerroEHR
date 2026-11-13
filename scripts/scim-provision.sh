#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Provisions the persons a development lane logs in as through FerroEHR's
# SCIM endpoint (RFC 7644 §3.3), so each login is bound to a demographic
# PERSON before the lane's run starts.
#
# A client provisions only under a scim_provision grant in the CDR. With
# SCIM_GRANT set, the script first grants the client through
# `ferroehr party-binding grant` on the operator's authority, since no
# administrator can log in before the persons it provisions exist; a client
# that holds the grant already gets nothing. It then mints an RFC 9068 client
# credentials access token with the committed SCIM test key
# (docs/conformance/party/scim/README.md: TEST MATERIAL, never production)
# using openssl alone, and creates each User with POST /scim/v2/Users. A User
# whose userName exists already (409 `uniqueness`) is taken as provisioned, so
# the step runs again on a kept volume.
#
# A Basic user is a credential alone: what it may do is a grant on its
# PERSON. With SCIM_GRANT set and the Basic users' binding issuer
# (urn:ferroehr:basic-user), each user is then granted, through
# `ferroehr party-binding grant --basic-user`, the scopes of every SMART on
# openEHR resource family with the product scope (`hold_scopes`) and its
# roles (`hold_roles`); a PERSON that holds the operation already gets
# nothing. No openEHR spec governs provisioning a lane: our own design.
#
# Usage:
#   scripts/scim-provision.sh <base-url> <client-id> <binding-issuer> [--name <formatted>] [--roles <roles>] <user>[=<login-key>]...
#
#   <base-url>   the server origin, such as http://localhost:8080
#   <client-id>  the client the token names
#   <binding-issuer>
#                the identifier issuer of every key this call provisions, the
#                binding_issuer of the client's scim_provision grant
#   <user>       the SCIM userName; <login-key> is the binding key the login
#                carries (loginIdentifier), the userName when omitted
#   --name       the User's name.formatted, the PERSON's name the server
#                writes as the committer it records (RFC 7643 §4.1.1); it
#                applies to every <user> after it, and a User without one has
#                no name
#   --roles      the space-separated roles each Basic user after it holds
#                (`hold_roles`), USER until it is given; an empty value
#                grants none
#
# Environment (each defaults to the dev lanes' SCIM issuer):
#   SCIM_KEY_FILE   the RS256 private key (docs/conformance/party/scim/scim-test.key.pem)
#   SCIM_ISSUER     the token's `iss` (https://scim.ferroehr.invalid)
#   SCIM_AUDIENCE   the token's `aud` (ferroehr-scim)
#   SCIM_SCOPE      the client's scope (ferroehr/scim)
#   SCIM_KID        the key id the JWKS names (scim-test)
#   SCIM_GRANT      the command that runs the server's binary against its
#                   database, such as `docker compose exec -T ferroehr ferroehr`;
#                   unset, the client is taken as granted already
set -euo pipefail

usage() {
  echo "usage: scim-provision.sh <base-url> <client-id> <binding-issuer> [--name <formatted>] [--roles <roles>] <user>[=<login-key>]..." >&2
  exit 2
}
[[ $# -ge 4 ]] || usage

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base_url="${1%/}"
client_id="$2"
binding_issuer="$3"
shift 3
key_file="${SCIM_KEY_FILE:-$root/docs/conformance/party/scim/scim-test.key.pem}"
issuer="${SCIM_ISSUER:-https://scim.ferroehr.invalid}"
audience="${SCIM_AUDIENCE:-ferroehr-scim}"
scope="${SCIM_SCOPE:-ferroehr/scim}"
kid="${SCIM_KID:-scim-test}"
extension="urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User"

if [[ -n "${SCIM_GRANT:-}" ]]; then
  read -ra grant <<<"$SCIM_GRANT"
  "${grant[@]}" party-binding grant --issuer "$issuer" --client-id "$client_id" \
    --operation scim_provision --element "binding_issuer=$binding_issuer" \
    --element "token_scope=$scope" >/dev/null
  echo "scim-provision: $client_id holds scim_provision for $binding_issuer"
fi

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

# The Basic users' binding issuer, whose users the script grants.
basic_users_issuer="urn:ferroehr:basic-user"
scopes="user/composition-*.cruds user/template-*.cruds user/aql-*.cruds __ferroehr.platform"

# hold <user> <roles>: grants the Basic user's PERSON the scopes and <roles>.
hold() {
  local user="$1" roles="$2" scope role
  local -a elements=()
  for scope in $scopes; do elements+=(--element "scope=$scope"); done
  "${grant[@]}" party-binding grant --basic-user "$user" --operation hold_scopes \
    "${elements[@]}" >/dev/null
  elements=()
  for role in $roles; do elements+=(--element "role=$role"); done
  if [[ ${#elements[@]} -gt 0 ]]; then
    "${grant[@]}" party-binding grant --basic-user "$user" --operation hold_roles \
      "${elements[@]}" >/dev/null
  fi
  echo "scim-provision: $user holds its scopes${roles:+ and $roles}"
}

formatted=""
roles="USER"
while [[ $# -gt 0 ]]; do
  if [[ "$1" == --name ]]; then
    [[ $# -ge 3 ]] || usage
    formatted="$2"
    shift 2
    continue
  fi
  if [[ "$1" == --roles ]]; then
    [[ $# -ge 3 ]] || usage
    roles="$2"
    shift 2
    continue
  fi
  entry="$1"
  shift
  user="${entry%%=*}"
  key="${entry#*=}"
  body="$(jq -cn --arg user "$user" --arg key "$key" --arg ext "$extension" \
    --arg formatted "$formatted" \
    '{schemas: ["urn:ietf:params:scim:schemas:core:2.0:User", $ext],
      userName: $user, ($ext): {loginIdentifier: $key}}
     + (if $formatted == "" then {} else {name: {formatted: $formatted}} end)')"
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
  if [[ -n "${SCIM_GRANT:-}" && "$binding_issuer" == "$basic_users_issuer" ]]; then
    hold "$user" "$roles"
  fi
done
