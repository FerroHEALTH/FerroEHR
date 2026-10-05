#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The OIDC / Keycloak probe family.
#
# Driven through the PUBLISHED quickstart overlay and the recipe the book gives
# the reader — a password-grant token fetch, then an API call with it. #2176 was
# exactly that recipe returning 401 forever, because the demo realm never put
# `ferroehr` in the token's `aud` while the same overlay required that audience.
# Two halves of one shipped file contradicting each other, and no test could see
# it because neither half is code.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

export KEYCLOAK_PORT="${PROBE_KC_PORT:-18081}"
KC="http://localhost:${KEYCLOAK_PORT}"
KC_REALM="$KC/auth/realms/ferroehr"

# The stack for this family: the quickstart PLUS the Keycloak overlay, which is
# how the book tells an operator to get an issuer.
oidc_up() {
  dc -f docker-compose.yml -f docker-compose.keycloak.yml up -d ferroehr keycloak >/dev/null 2>&1
}

# The middle segment of a JWT, decoded. `base64 -d` wants padding, and a JWT is
# base64url without it — so pad to a multiple of 4 and translate the alphabet.
jwt_claims() {
  local token="$1" payload
  payload="$(printf '%s' "$token" | cut -d. -f2 | tr '_-' '/+')"
  case $(( ${#payload} % 4 )) in
    2) payload="${payload}==" ;;
    3) payload="${payload}=" ;;
    *) ;;
  esac
  printf '%s' "$payload" | base64 -d 2>/dev/null
}

probes_oidc() {
  bold "OIDC (Keycloak)"

  oidc_up
  if ! wait_http "$KC_REALM/.well-known/openid-configuration" 150; then
    probe "P-OIDC-UP" "working" "compose" "-" "the quickstart overlay starts an issuer"
    probe_fail "a Keycloak realm answering discovery" "no response after 300s" \
      "the overlay's own healthcheck gates the CDR, so this is the overlay, not the realm"
    probe_done
    return 0
  fi
  wait_http "$CDR/health/readiness" 120 || true

  # The documented recipe, verbatim: fetch a token with the password grant.
  local token
  token="$(curl -s -d client_id=ferroehr -d client_secret=ferroehr-quickstart-secret \
    -d username=ferroehr -d password=ferroehr -d grant_type=password \
    "$KC_REALM/protocol/openid-connect/token" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"

  # #2176, far end: the CLAIM, not the status code. The realm must actually put
  # this server in the audience — a token that omits it is refused by every
  # correct resource server (RFC 7519 §4.1.3), which is what made the shipped
  # quickstart impossible rather than merely misconfigured.
  probe "P-OIDC-AUD" "working" "compose" "#2176" \
    "the demo realm mints a token carrying aud=ferroehr"
  if [[ -z "$token" ]]; then
    probe_fail "an access token from the password grant" "the token endpoint returned none" \
      "the realm's client must have the password grant enabled"
  else
    # `aud` is a string when there is one audience and an ARRAY when there are
    # several — RFC 7519 §4.1.3 permits both, and Keycloak emits either
    # depending on which mappers fire. `[.aud] | flatten` normalizes the two
    # into one list, which is why this reads the claim with jq rather than a
    # regex: the first attempt matched only the string spelling, and its BRE
    # alternation silently produced nothing on BSD sed anyway.
    local aud
    aud="$(jwt_claims "$token" | jq -r '[.aud] | flatten | join(",")' 2>/dev/null)"
    assert_contains "$aud" 'ferroehr' \
      "without an audience mapper the realm cannot mint a token this server accepts"
  fi
  probe_done

  # And the whole point: the documented flow SUCCEEDS.
  probe "P-OIDC-ACCEPT" "working" "server" "#2176" \
    "the API accepts that token — the documented quickstart works end to end"
  if [[ -z "$token" ]]; then
    probe_fail "201 from POST /ehr" "no token to present"
  else
    local code
    code="$(curl -s -o /dev/null -w '%{http_code}' -X POST \
      -H "Authorization: Bearer $token" "$API/ehr")"
    assert_eq "201" "$code" \
      "a token the issuer minted for this audience must be accepted"
  fi
  probe_done

  # The one spec-grounded rule in the whole access layer, observed on the wire
  # rather than inferred: a request with NO credential is 401 and the response
  # must carry a challenge naming a scheme the server implements (RFC 9110
  # §11.6.1 — a challenge naming nothing is what makes a 401 unactionable).
  probe "P-OIDC-CHALLENGE" "working" "server" "-" \
    "no credential: 401 with a WWW-Authenticate challenge naming Bearer"
  local hdrs
  hdrs="$(curl -s -o /dev/null -D - -X POST "$API/ehr")"
  assert_contains "$hdrs" "401" "an unauthenticated write must be refused"
  # Header names are case-insensitive on the wire, so fold before matching.
  assert_contains "$(printf '%s' "$hdrs" | tr '[:upper:]' '[:lower:]')" "www-authenticate" \
    "a 401 without a challenge tells the client nothing about how to authenticate"
  assert_contains "$(printf '%s' "$hdrs" | tr '[:upper:]' '[:lower:]')" "bearer" \
    "with an issuer configured the challenge must advertise Bearer"
  probe_done

  # A garbage bearer is refused — the negative twin, so the probe above cannot
  # pass because authentication is off altogether.
  probe "P-OIDC-REFUSE" "working" "server" "-" \
    "a bearer token that is not the issuer's is refused"
  assert_eq "401" "$(http_code -X POST -H 'Authorization: Bearer not-a-token' "$API/ehr")" \
    "acceptance means nothing if an invalid token is also accepted"
  probe_done
}

# A token for one of the demo identities, or empty if the grant is refused.
oidc_token_for() {
  curl -s -d client_id=ferroehr -d client_secret=ferroehr-quickstart-secret \
    -d "username=$1" -d "password=$2" -d grant_type=password \
    "$KC_REALM/protocol/openid-connect/token" \
    | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p'
}

# Role separation, which is the half #2160 asks for and the half a single-user
# realm cannot show.
#
# The distinction being measured is the ONE spec-grounded rule in the access
# layer: 401 means the credential was not accepted, 403 means it was and the
# caller still may not do this. A deployment where every authenticated caller
# may do everything can never produce the second, so the demo realm carries
# four identities and this overlay turns RBAC on.
probes_oidc_roles() {
  bold "OIDC role separation (four identities, RBAC on)"

  local admin clinician auditor nobody
  admin="$(oidc_token_for ferroehr ferroehr)"
  clinician="$(oidc_token_for clinician clinician)"
  auditor="$(oidc_token_for auditor auditor)"
  nobody="$(oidc_token_for nobody nobody)"

  probe "P-OIDC-IDENTITIES" "working" "compose" "#2160" \
    "the demo realm mints a token for each of the four identities"
  local missing=""
  [[ -n "$admin" ]]     || missing="$missing ferroehr"
  [[ -n "$clinician" ]] || missing="$missing clinician"
  [[ -n "$auditor" ]]   || missing="$missing auditor"
  [[ -n "$nobody" ]]    || missing="$missing nobody"
  if [[ -n "$missing" ]]; then
    probe_fail "a token for every demo user" "no token for:$missing" \
      "the realm import must create all four, or the separation below is untestable"
    probe_done
    return 0
  fi
  probe_done

  # A clinical role may write clinical data.
  probe "P-OIDC-CLINICAL" "working" "server" "#2160" \
    "USER may create an EHR"
  assert_eq "201" "$(http_code -X POST -H "Authorization: Bearer $clinician" "$API/ehr")" \
    "the clinical role must reach the clinical API, or the roles are simply wrong"
  probe_done

  # ...and may NOT reach the admin surface. This is the 403 that a
  # single-user, RBAC-off quickstart could never produce.
  probe "P-OIDC-CLINICAL-DENIED" "broken" "server" "#2160" \
    "USER is REFUSED the admin surface with 403, not 401"
  local code
  # The admin group is nested UNDER the API base path, not beside it. An
  # earlier version of this probe used /ferroehr/rest/admin/... and got a 404
  # from the router — which looks like a refusal and proves nothing about
  # authorization.
  code="$(http_code -X DELETE -H "Authorization: Bearer $clinician" \
          "$API/admin/ehr/00000000-0000-0000-0000-000000000000")"
  case "$code" in
    403) : ;;
    401) probe_fail "403" "$code" \
           "401 says the credential was rejected; this credential is valid and merely unauthorized" ;;
    404) probe_fail "403" "$code" \
           "a 404 here means the router or the lookup answered before authorization did; an unauthorized caller must not learn whether the object exists" ;;
    *)   probe_fail "403" "$code" \
           "an authenticated caller without the admin role must be refused, not served" ;;
  esac
  probe_done

  # READONLY overrides a grant it otherwise holds. The auditor carries USER,
  # so without the override this write would succeed.
  probe "P-OIDC-READONLY" "broken" "server" "#2160" \
    "READONLY overrides the USER grant: the write is refused 403"
  code="$(http_code -X POST -H "Authorization: Bearer $auditor" "$API/ehr")"
  assert_eq "403" "$code" \
    "READONLY is documented to override any grant, and this user holds USER"
  probe_done

  probe "P-OIDC-READONLY-READS" "working" "server" "#2160" \
    "the same READONLY identity may still read"
  assert_eq "200" "$(http_code -H "Authorization: Bearer $auditor" "$CDR/ferroehr/rest/status")" \
    "read-only must mean read-only, not no access at all"
  probe_done

  # Authenticated with no roles at all: still 403, never 401.
  probe "P-OIDC-NOROLES" "broken" "server" "#2160" \
    "a valid token carrying no roles is refused 403, not 401"
  code="$(http_code -X POST -H "Authorization: Bearer $nobody" "$API/ehr")"
  case "$code" in
    403) : ;;
    401) probe_fail "403" "$code" \
           "the token is valid, so the refusal is authorization and must not masquerade as authentication" ;;
    *)   probe_fail "403" "$code" "a role-less caller must be refused" ;;
  esac
  probe_done

  # A malformed Authorization header is a 400, not a 401: the server never read
  # a credential, so there is nothing to challenge.
  probe "P-OIDC-MALFORMED" "broken" "server" "-" \
    "a malformed Authorization header is 400, not 401"
  assert_eq "400" "$(http_code -X POST -H 'Authorization: NotAScheme' "$API/ehr")" \
    "a 401 here would claim the credential was rejected when none was ever parsed"
  probe_done
}
