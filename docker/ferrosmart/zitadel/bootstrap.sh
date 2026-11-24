#!/bin/sh
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Bootstraps ZITADEL v4.20.5 as FerroSMART's upstream login for
# docker-compose.ferrosmart-zitadel.yml, through ZITADEL's v2 APIs with the
# IAM_OWNER token its first instance wrote (`cmd/setup/steps.yaml`,
# FirstInstance), as FerroSMART documents the setup:
# https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/docs/architecture.md#1320-upstream-providers-zitadel
#
#   project      ferrosmart, role assertion on, the roles CLINICIAN and
#                PATIENT (project/v2 CreateProject, AddProjectRole)
#   application  ferrosmart-login: an OIDC web app, the code flow,
#                Private Key JWT, the redirect URI FERROSMART_ISSUER/callback,
#                roles asserted into the ID Token, and its key
#                (application/v2 CreateApplication, CreateApplicationKey)
#   user         clinician, holding CLINICIAN (user/v2 AddHumanUser;
#                authorization/v2 CreateAuthorization)
#
# ZITADEL generates the OIDC client id and the application's key with its key
# id, so the script writes OUT/upstream-assertion.pem and OUT/ferrosmart.toml:
# FerroSMART's configuration BASE_CONFIG with the [[ferrosmart.upstream]]
# tables that name them appended. FerroEHR binds a person by FerroSMART's
# pairwise `sub`, so no `fhirUser` claim and no Actions target are set up. A
# second run finds OUT/ferrosmart.toml and does nothing. It needs sh, curl,
# sed and base64, so it runs in the curl image. No specification governs the
# script: our own design.
#
# Environment: ZITADEL_API, ADMIN_PAT_FILE, USER_PASSWORD_FILE,
# FERROSMART_ISSUER, BASE_CONFIG, OUT.
set -eu

: "${ZITADEL_API:?}" "${ADMIN_PAT_FILE:?}" "${USER_PASSWORD_FILE:?}" "${FERROSMART_ISSUER:?}"
: "${BASE_CONFIG:?}" "${OUT:?}"

if [ -f "$OUT/ferrosmart.toml" ]; then
  echo "bootstrap: $OUT/ferrosmart.toml exists; ZITADEL is bootstrapped"
  exit 0
fi

fail() {
  echo "bootstrap: $*" >&2
  exit 1
}

# The string member `$2` of the JSON object `$1`, which holds no escape.
member() {
  printf '%s' "$1" | sed -n "s/.*\"$2\" *: *\"\\([^\"]*\\)\".*/\\1/p"
}

# Calls the v2 API method `$1` with the JSON body `$2` (Connect, unary JSON).
call() {
  answer=$(curl -sS -X POST -H "Authorization: Bearer $(cat "$ADMIN_PAT_FILE")" \
    -H 'Content-Type: application/json' -w '\n%{http_code}' "$ZITADEL_API/$1" -d "$2")
  status=$(printf '%s' "$answer" | tail -n 1)
  body=$(printf '%s' "$answer" | sed '$d')
  [ "$status" = 200 ] || fail "$1 answered $status: $body"
  printf '%s' "$body"
}

i=0
until curl -fsS -o /dev/null "$ZITADEL_API/debug/ready"; do
  i=$((i + 1))
  [ "$i" -lt 90 ] || fail "ZITADEL is not ready at $ZITADEL_API"
  sleep 2
done
i=0
until [ -s "$ADMIN_PAT_FILE" ]; do
  i=$((i + 1))
  [ "$i" -lt 90 ] || fail "no admin token at $ADMIN_PAT_FILE"
  sleep 2
done

org=$(member "$(call zitadel.org.v2.OrganizationService/ListOrganizations '{}')" id)
[ -n "$org" ] || fail "no organization"

call zitadel.project.v2.ProjectService/CreateProject \
  "{\"organizationId\":\"$org\",\"projectId\":\"ferrosmart\",\"name\":\"FerroSMART\",\"projectRoleAssertion\":true}" > /dev/null
for role in CLINICIAN PATIENT; do
  call zitadel.project.v2.ProjectService/AddProjectRole \
    "{\"projectId\":\"ferrosmart\",\"roleKey\":\"$role\",\"displayName\":\"$role\"}" > /dev/null
done

app=$(call zitadel.application.v2.ApplicationService/CreateApplication "{
  \"projectId\":\"ferrosmart\",\"applicationId\":\"ferrosmart-login\",\"name\":\"FerroSMART\",
  \"oidcConfiguration\":{
    \"redirectUris\":[\"$FERROSMART_ISSUER/callback\"],
    \"responseTypes\":[\"OIDC_RESPONSE_TYPE_CODE\"],
    \"grantTypes\":[\"OIDC_GRANT_TYPE_AUTHORIZATION_CODE\"],
    \"applicationType\":\"OIDC_APP_TYPE_WEB\",
    \"authMethodType\":\"OIDC_AUTH_METHOD_TYPE_PRIVATE_KEY_JWT\",
    \"developmentMode\":true,
    \"idTokenRoleAssertion\":true,
    \"idTokenUserinfoAssertion\":true}}")
client_id=$(member "$app" clientId)
[ -n "$client_id" ] || fail "the application has no client id"
# NOTE: application/v2 CreateApplicationKeyResponse.key_details, base64 JSON with `keyId` and the PEM `key`.
key=$(call zitadel.application.v2.ApplicationService/CreateApplicationKey \
  '{"projectId":"ferrosmart","applicationId":"ferrosmart-login","expirationDate":"2099-01-01T00:00:00Z"}')
details=$(printf '%s' "$(member "$key" keyDetails)" | base64 -d)
key_id=$(member "$details" keyId)
pem=$(member "$details" key | sed 's/\\n/\n/g')
[ -n "$key_id" ] && [ -n "$pem" ] || fail "the application key has no id or private key"

password=$(cat "$USER_PASSWORD_FILE")
call zitadel.user.v2.UserService/AddHumanUser "{
  \"userId\":\"clinician\",\"username\":\"clinician\",\"organization\":{\"orgId\":\"$org\"},
  \"profile\":{\"givenName\":\"Clara\",\"familyName\":\"Clinician\"},
  \"email\":{\"email\":\"clinician@example.org\",\"isVerified\":true},
  \"password\":{\"password\":\"$password\",\"changeRequired\":false}}" > /dev/null
call zitadel.authorization.v2.AuthorizationService/CreateAuthorization \
  "{\"userId\":\"clinician\",\"projectId\":\"ferrosmart\",\"organizationId\":\"$org\",\"roleKeys\":[\"CLINICIAN\"]}" > /dev/null

mkdir -p "$OUT"
printf '%s\n' "$pem" > "$OUT/upstream-assertion.pem"
printf '%s' "$org" > "$OUT/organization"
{
  cat "$BASE_CONFIG"
  cat << TOML

# Written by docker/ferrosmart/zitadel/bootstrap.sh: ZITADEL as the upstream
# login (FerroSMART docs/architecture.md §13.20).
[[ferrosmart.upstream]]
name = "zitadel"

[ferrosmart.upstream.oidc]
issuer = "$ZITADEL_API"
client_id = "$client_id"
scopes = ["openid"]
role_claims = ["urn:zitadel:iam:org:project:ferrosmart:roles"]
# ZITADEL names the project beside the client in every ID Token's aud.
trusted_audiences = ["ferrosmart"]

[ferrosmart.upstream.oidc.private_key_jwt]
algorithm = "RS256"

[ferrosmart.upstream.oidc.private_key_jwt.keys]
private_key_file = "$OUT/upstream-assertion.pem"
kid = "$key_id"
TOML
} > "$OUT/ferrosmart.toml"
# NOTE: FerroSMART runs as another user, so it reads the files the script wrote.
chmod 0644 "$OUT/upstream-assertion.pem" "$OUT/ferrosmart.toml" "$OUT/organization"
echo "bootstrap: ZITADEL is bootstrapped; the client id is $client_id"
