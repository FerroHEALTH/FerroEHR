#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The hospital scenario's identities behind FerroSMART (#5832).
#
# FerroSMART, the family's SMART App Launch 2.x authorization server, issues
# every token the scenario presents to FerroEHR; the upstream provider
# PROBE_HOSPITAL_UPSTREAM names (`keycloak`, the default, or `zitadel`) serves
# only the login. A professional's or a patient's token comes from the
# authorization code flow with PKCE of a public SMART app, `hospital-app`,
# through the provider's login, FerroSMART's patient picker and its consent
# page, as a browser without script takes them (SMART App Launch 2.2.0,
# Obtain authorization code). A system client's token comes from the client
# credentials grant with a `private_key_jwt` assertion (SMART App Launch
# 2.2.0, Backend Services). The person a token names binds to FerroEHR by
# FerroSMART's pairwise subject (FerroSMART docs/architecture.md §13.6 item 9).
#
# Sourced by scripts/deploy-probe.sh before hospital.sh; never run directly.
# No specification governs the driver: our own design.

HS_UPSTREAM="${PROBE_HOSPITAL_UPSTREAM:-keycloak}"
HS_ISSUER="http://localhost:8280"
HS_BASE="http://localhost:8080/ferroehr/rest/openehr/v1"
HS_APP="hospital-app"
HS_REDIRECT="http://localhost:9999/callback"
# shellcheck disable=SC2034 # read by hospital.sh
HS_LAUNCH_URL="http://localhost:9999/launch"
HS_KC="http://localhost:8380"
HS_KC_ISSUER="$HS_KC/realms/ferroehr"
HS_ZITADEL="http://localhost:8480"
# The binding issuer of FerroSMART's subjects, as docker-compose.ferrosmart.yml
# names it in FerroSMART's party table.
HS_LOGIN_ISSUER="urn:example:ferrosmart-login"
# The system clients, each a FerroSMART Backend Services client with its own
# key; `hospital-scim` provisions the people.
HS_SYSTEMS="lab-system hie-member hie-outside access-service hospital-scim"
# The scopes a professional's token asks for. The product scope reaches the
# ADMIN API (FerroSMART grants it a person through a role, FerroHEALTH/FerroSMART#647).
HS_USER_SCOPE="openid user/composition-*.cruds user/aql-*.cruds user/template-*.r __ferroehr.platform"
# shellcheck disable=SC2034 # read by hospital.sh
HS_SYSTEM_SCOPE="system/composition-*.cruds system/aql-*.cruds system/template-*.r"
HS_PATIENT_SCOPE="openid launch/patient patient/composition-*.crs patient/aql-*.rs"

hs_dir() { printf '%s' "$PROBE_TMP/ferrosmart"; }

hs_b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

# FerroSMART's subject of the provider's user `sub`: base64url of the SHA-256
# of the provider's issuer, a zero octet and `sub` (FerroSMART
# `app/ferrosmart/src/upstream.rs`, `pairwise`).
hs_pairwise() {
  local issuer
  case "$HS_UPSTREAM" in
    zitadel) issuer="$HS_ZITADEL" ;;
    *) issuer="$HS_KC_ISSUER" ;;
  esac
  printf '%s\0%s' "$issuer" "$1" | openssl dgst -sha256 -binary | hs_b64url
}

# The password of the provider's user `user`, which ZITADEL's policy holds to
# an upper case letter, a digit and a symbol.
hs_password() { printf 'A1!%s-hospital' "$1"; }

# The FerroSMART configuration of the scenario, with the subject namespace
# `namespace` its picker looks EHRs up under.
hs_config() {
  local namespace="$1" role platform
  {
    cat <<TOML
# Written by scripts/deploy-probes/hospital_smart.sh for one run.
[server]
listen = "0.0.0.0:8280"

[ferrosmart]
issuer = "$HS_ISSUER"
assurance_minimum = "none"

[ferrosmart.database]
url_file = "/run/secrets/ferrosmart_database_url"
max_connections = 8

[ferrosmart.keys]
private_key_files = ["/run/secrets/ferrosmart_signing_key"]

[ferrosmart.backend]
algorithm = "ES384"

[ferrosmart.backend.keys]
private_key_file = "/run/secrets/ferrosmart_backend_key"

[[ferrosmart.resource]]
base = "$HS_BASE"
kind = "openehr"
subject_namespace = "$namespace"

TOML
    # Every role the people hold is named, so FerroSMART carries it in the
    # token's `roles` (FerroHEALTH/FerroSMART#649); ADMIN alone grants the
    # product scope of the ADMIN API (FerroHEALTH/FerroSMART#647).
    for role in CLINICIAN NURSE EMERGENCY ADMIN; do
      platform=""
      if [[ "$role" == ADMIN ]]; then platform=', "__ferroehr.platform"'; fi
      printf '[[ferrosmart.role]]\nname = "%s"\nscopes = ["user/composition-*.cruds", "user/aql-*.cruds", "user/template-*.r", "patient/composition-*.rs"%s]\n\n' "$role" "$platform"
    done
    cat <<TOML
[[ferrosmart.role]]
name = "PATIENT"
scopes = ["patient/composition-*.crs", "patient/aql-*.rs"]
TOML
  } > "$(hs_dir)/ferrosmart.toml"
}

# The client documents of the scenario, which the overlay's
# `ferrosmart-clients` one-shot applies beside the viewer's.
hs_documents() {
  local dir client scope
  dir="$(hs_dir)/documents"
  jq -n --arg id "$HS_APP" --arg redirect "$HS_REDIRECT" --arg base "$HS_BASE" '{
    client_id: $id,
    client_name: "Hospital app",
    token_endpoint_auth_method: "none",
    redirect_uris: [$redirect],
    scope: "openid launch launch/patient user/composition-*.cruds user/aql-*.cruds user/template-*.r patient/composition-*.crs patient/aql-*.rs patient/composition-*.rs patient/composition-*.r __ferroehr.platform",
    grant_types: ["authorization_code"],
    resources: [$base]
  }' > "$dir/$HS_APP.json"
  for client in $HS_SYSTEMS; do
    scope="system/composition-*.cruds system/aql-*.cruds system/template-*.r"
    [[ "$client" == hospital-scim ]] && scope="ferroehr/scim"
    jq -n --arg id "$client" --arg scope "$scope" --arg base "$HS_BASE" \
      --slurpfile jwks "$(hs_dir)/clients/$client.jwks.json" '{
      client_id: $id,
      client_name: $id,
      token_endpoint_auth_method: "private_key_jwt",
      jwks: $jwks[0],
      scope: $scope,
      grant_types: ["client_credentials"],
      resources: [$base]
    }' > "$dir/$client.json"
  done
}

# The `ferrosmart client apply` arguments of the scenario's documents, as the
# one-shot reads them, the viewer's last.
hs_document_paths() {
  local client
  printf '/etc/ferrosmart/clients/%s.json, ' "$HS_APP"
  for client in $HS_SYSTEMS; do printf '/etc/ferrosmart/clients/%s.json, ' "$client"; done
  printf '/run/secrets/ferrosmart_viewer_client'
}

# The secrets, the system clients' keys and the configuration of a fresh run.
hs_prepare() {
  local namespace="$1" dir client modulus
  dir="$(hs_dir)"
  bash docker/ferrosmart/secrets.sh "$dir" >/dev/null || return 1
  [[ -s "$dir/zitadel_masterkey" ]] || openssl rand -hex 16 | tr -d '\n' > "$dir/zitadel_masterkey"
  [[ -s "$dir/zitadel_user_password" ]] || hs_password clinician > "$dir/zitadel_user_password"
  # A fresh stack starts ZITADEL on an empty volume, whose bootstrap must run
  # again: it keeps an out/ferrosmart.toml it finds, which would carry the last
  # stack's ZITADEL client and the last run's resource base.
  [[ -d "$dir/zitadel" ]] && find "$dir/zitadel" -mindepth 1 -delete
  mkdir -p "$dir/clients" "$dir/documents" "$dir/zitadel" "$dir/tokens"
  for client in $HS_SYSTEMS; do
    [[ -s "$dir/clients/$client.pem" ]] && continue
    openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$dir/clients/$client.pem" 2>/dev/null
    modulus="$(openssl rsa -in "$dir/clients/$client.pem" -noout -modulus | cut -d= -f2 | xxd -r -p | hs_b64url)"
    jq -n --arg n "$modulus" --arg k "$client" \
      '{keys: [{kty: "RSA", kid: $k, alg: "RS384", use: "sig", n: $n, e: "AQAB"}]}' > "$dir/clients/$client.jwks.json"
  done
  hs_config "$namespace"
  hs_documents
  chmod 0755 "$dir/clients" "$dir/documents" "$dir/zitadel"
  chmod 0644 "$dir"/*.pem "$dir"/*.json "$dir/clients"/* "$dir"/zitadel_* "$dir/database_url" "$dir/postgres_password" "$dir/documents"/*
  # A fresh stack forgets every grant, so no token of the last one is kept.
  rm -f "$dir/tokens/"*
}

# The compose files of the upstream.
hs_upstream_files() {
  printf '%s\n' -f docker-compose.ferrosmart.yml -f "docker-compose.ferrosmart-$HS_UPSTREAM.yml"
}

# The services of the upstream the scenario starts.
hs_upstream_services() {
  case "$HS_UPSTREAM" in
    zitadel) printf '%s' "ferrosmart-zitadel ferrosmart-zitadel-login" ;;
    *) printf '%s' "ferrosmart-keycloak" ;;
  esac
}

# Exports what the FerroSMART overlays read for this run.
hs_export() {
  local namespace="${1:-demographic}"
  export FERROSMART_SECRETS FERROSMART_CONFIG_FILE FERROSMART_CLIENTS_DIR FERROSMART_ZITADEL_DIR FERROSMART_ZITADEL_UID
  # docker-compose.ferrosmart.yml sets the picker's namespace in FerroSMART's
  # environment, which outranks the file, so the scenario's goes there too.
  export FERROSMART_SUBJECT_NAMESPACE="$namespace"
  FERROSMART_SECRETS="$(hs_dir)"
  FERROSMART_CONFIG_FILE="$(hs_dir)/ferrosmart.toml"
  FERROSMART_CLIENTS_DIR="$(hs_dir)/documents"
  FERROSMART_ZITADEL_DIR="$(hs_dir)/zitadel"
  FERROSMART_ZITADEL_UID="$(id -u):$(id -g)"
}

# ── The provider's people ─────────────────────────────────────────────────────

# A master-realm administrator token of Keycloak.
hs_kc_admin() {
  curl -s -d client_id=admin-cli -d username="${KEYCLOAK_ADMIN_USER:-admin}" \
    -d password="${KEYCLOAK_ADMIN_PASSWORD:-admin}" -d grant_type=password \
    "$HS_KC/realms/master/protocol/openid-connect/token" | jq -r '.access_token // empty'
}

# Calls ZITADEL's v2 API method `$1` with the JSON body `$2` as its administrator.
hs_zitadel() {
  curl -s -X POST -H "Authorization: Bearer $(cat "$(hs_dir)/zitadel/admin.pat")" \
    -H 'Content-Type: application/json' "$HS_ZITADEL/$1" -d "$2"
}

# Creates the provider's role `role`; one that exists is kept.
hs_role() {
  local role="$1"
  case "$HS_UPSTREAM" in
    zitadel)
      hs_zitadel zitadel.project.v2.ProjectService/AddProjectRole \
        "{\"projectId\":\"ferrosmart\",\"roleKey\":\"$role\",\"displayName\":\"$role\"}" >/dev/null ;;
    *)
      curl -s -o /dev/null -X POST -H "Authorization: Bearer $(hs_kc_admin)" \
        -H 'Content-Type: application/json' -d "{\"name\":\"$role\"}" "$HS_KC/admin/realms/ferroehr/roles" ;;
  esac
}

# Creates the provider's user `user` named `first last` holding the roles
# given after, and records their FerroSMART subject as HV_sub_<user>.
hs_user() {
  local user="$1" first="$2" last="$3" password sub
  shift 3
  password="$(hs_password "$user")"
  case "$HS_UPSTREAM" in
    zitadel)
      local org roles
      org="$(cat "$(hs_dir)/zitadel/out/organization")"
      hs_zitadel zitadel.user.v2.UserService/AddHumanUser "{\"userId\":\"$user\",\"username\":\"$user\",
        \"organization\":{\"orgId\":\"$org\"},\"profile\":{\"givenName\":\"$first\",\"familyName\":\"$last\"},
        \"email\":{\"email\":\"$user@example.test\",\"isVerified\":true},
        \"password\":{\"password\":\"$password\",\"changeRequired\":false}}" >/dev/null
      roles="$(printf '%s\n' "$@" | jq -Rnc '[inputs | select(length > 0)]')"
      [[ "$roles" = "[]" ]] || hs_zitadel zitadel.authorization.v2.AuthorizationService/CreateAuthorization \
        "{\"userId\":\"$user\",\"projectId\":\"ferrosmart\",\"organizationId\":\"$org\",\"roleKeys\":$roles}" >/dev/null
      sub="$user" ;;
    *)
      local token role roles
      token="$(hs_kc_admin)"
      curl -s -o /dev/null -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
        -d "$(jq -nc --arg u "$user" --arg f "$first" --arg l "$last" --arg p "$password" \
          '{username:$u,enabled:true,firstName:$f,lastName:$l,email:($u+"@example.test"),emailVerified:true,
            credentials:[{type:"password",value:$p,temporary:false}]}')" \
        "$HS_KC/admin/realms/ferroehr/users"
      sub="$(curl -s -H "Authorization: Bearer $token" \
        "$HS_KC/admin/realms/ferroehr/users?username=$user&exact=true" | jq -r '.[0].id // empty')"
      roles="[]"
      for role in "$@"; do
        roles="$(jq -c --argjson r "$(curl -s -H "Authorization: Bearer $token" \
          "$HS_KC/admin/realms/ferroehr/roles/$role")" '. + [$r]' <<<"$roles")"
      done
      [[ "$roles" = "[]" ]] || curl -s -o /dev/null -X POST -H "Authorization: Bearer $token" \
        -H 'Content-Type: application/json' -d "$roles" "$HS_KC/admin/realms/ferroehr/users/$sub/role-mappings/realm" ;;
  esac
  [[ -n "$sub" ]] || return 1
  hv_set "sub_$user" "$(hs_pairwise "$sub")"
}

# ── Tokens ────────────────────────────────────────────────────────────────────

# Prints the token cached as `key` while it has a minute left.
hs_cached() {
  local file
  file="$(hs_dir)/tokens/$1"
  [[ -s "$file" ]] || return 1
  [[ "$(sed -n 1p "$file")" -gt "$(( $(date +%s) + 60 ))" ]] || return 1
  sed -n 2p "$file"
}

# Caches the token response `answer` as `key` and prints its access token.
hs_keep() {
  local key="$1" answer="$2" token expires
  token="$(jq -r '.access_token // empty' <<<"$answer" 2>/dev/null)"
  if [[ -z "$token" ]]; then
    echo "      FerroSMART answered no token for $key: ${answer:0:300}" >&2
    if [[ "$answer" == *invalid_target* ]]; then
      echo "        the resource asked for: $HS_BASE; the one its configuration serves:" \
        "$(grep -E '^base' "$(hs_dir)/ferrosmart.toml" 2>/dev/null)" >&2
      echo "        FerroSMART's container: $(hdc ps --format '{{.Name}} {{.CreatedAt}} {{.Status}}' ferrosmart 2>&1)" >&2
      hdc logs --no-color --tail 15 ferrosmart 2>&1 | sed 's/^/        /' >&2
    fi
    return 1
  fi
  expires="$(jq -r '.expires_in // 60' <<<"$answer")"
  printf '%s\n%s\n' "$(( $(date +%s) + expires ))" "$token" > "$(hs_dir)/tokens/$key"
  printf '%s' "$answer" > "$(hs_dir)/tokens/$key.response"
  printf '%s' "$token"
}

# A Backend Services token of the system client `client` for `scope`
# (SMART App Launch 2.2.0, Backend Services): a client credentials grant
# authenticated by an RS384 assertion its key signs, `aud` the token URL.
hs_system_token() {
  local client="$1" scope="$2" now header claims input signature answer
  hs_cached "system-$client" && return 0
  now="$(date +%s)"
  header="$(jq -cn --arg k "$client" '{alg:"RS384",typ:"JWT",kid:$k}' | hs_b64url)"
  claims="$(jq -cn --arg c "$client" --arg a "$HS_ISSUER/token" --argjson n "$now" --arg j "$(openssl rand -hex 16)" \
    '{iss:$c,sub:$c,aud:$a,exp:($n+60),iat:$n,jti:$j}' | hs_b64url)"
  input="$header.$claims"
  signature="$(printf '%s' "$input" | openssl dgst -sha384 -sign "$(hs_dir)/clients/$client.pem" | hs_b64url)"
  answer="$(curl -s "$HS_ISSUER/token" --data-urlencode grant_type=client_credentials \
    --data-urlencode "scope=$scope" --data-urlencode "resource=$HS_BASE" \
    --data-urlencode client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer \
    --data-urlencode "client_assertion=$input.$signature")"
  hs_keep "system-$client" "$answer"
}

# The value of the attribute `$3` in the first tag of `$1` that `$2` matches,
# with the HTML entities of a URL decoded.
hs_attribute() {
  grep -oE "$2" <<<"$1" | sed -n 1p | grep -oE "$3=\"[^\"]*\"" | sed -n 1p |
    sed -E "s/^$3=\"//; s/\"$//; s/&amp;/\\&/g; s/&#x2F;/\\//g; s/&#47;/\\//g; s/&#61;/=/g"
}

# The query parameter `$1` of the URL `$2`, decoded.
hs_param() {
  local value
  value="$(tr '&' '\n' <<<"${2#*\?}" | sed -n "s/^$1=//p" | sed -n 1p)"
  value="${value//+/ }"
  printf '%b' "${value//%/\\x}"
}

# The location the request answers with, or nothing; the status and the page
# of the answer stay beside the cookie jar for hs_answered.
hs_redirect() {
  local out
  out="$(curl -sS -c "$HS_JAR" -b "$HS_JAR" -o "$HS_JAR.page" -w '%{http_code} %{redirect_url}' "$@")"
  printf '%s' "${out%% *}" > "$HS_JAR.status"
  printf '%s' "${out#* }"
}

# Says what the last request of hs_redirect answered when it sent the
# browser nowhere: its status, the page's title and message, and FerroSMART's
# last log lines.
hs_answered() {
  local page
  page="$(tr -s '\n' ' ' < "$HS_JAR.page" 2>/dev/null)"
  echo "      FerroSMART answered $(cat "$HS_JAR.status" 2>/dev/null): $(sed -nE 's/.*<title>([^<]*)<\/title>.*/\1/p' <<<"$page")" >&2
  echo "        $(sed -E 's/.*<body[^>]*>//; s/<[^>]*>/ /g; s/ +/ /g' <<<"$page" | cut -c1-400)" >&2
  if [[ -s "$HS_JAR.picker" ]]; then
    echo "        the picker page before it: $(tr -s '\n' ' ' < "$HS_JAR.picker" | sed -E 's/.*<body[^>]*>//; s/<[^>]*>/ /g; s/ +/ /g' | cut -c1-400)" >&2
  fi
  # Every refusal both instances logged in the last two minutes, whatever its
  # route: the picker's lookup reaches whichever layer answers it.
  hdc logs --no-color --since 2m ferroehr ferroehr-2 2>&1 \
    | sed -E $'s/\x1b\\[[0-9;]*m//g' \
    | grep -E 'status=4[0-9][0-9]|refused|denied' | sed -E 's/request\{[^}]*\}: //' \
    | tail -n 12 | cut -c1-400 | sed 's/^/        /' >&2
  hs_lookup_records
  hdc logs --no-color --tail 15 ferrosmart 2>&1 | sed 's/^/        /' >&2
}

# Prints the access record of each EHR lookup by subject either instance
# refused in the last two minutes: it names the caller's standing and the
# ground of a refusal the answer itself withholds.
hs_lookup_records() {
  local rid
  for rid in $(hdc logs --no-color --since 2m ferroehr ferroehr-2 2>&1 \
    | sed -E $'s/\x1b\\[[0-9;]*m//g' \
    | grep -E 'route="/ferroehr/rest/openehr/v1/ehr" status=4' \
    | sed -nE 's/.*request_id="([^"]+)".*/\1/p' | tail -n 2); do
    h_call 1 p:hans GET "$HX/admin/access-log?request_id=$rid"
    echo "        the access record of lookup $rid ($H_CODE): $(jq -c '[.entries[]? | del(.uid, .link,
      .system_id, .time_recorded, .action, .operation, .accessor, .retention, .channel)]' <<<"$H_BODY" 2>/dev/null | cut -c1-3000)" >&2
  done
}

# Takes the person `user` through FerroSMART's authorization request `url`:
# the provider's login, FerroSMART's picker (choosing the EHR `ehr` of the
# subject `subject`) when it shows one, and its consent page, approving every
# scope offered. Prints the redirect back to the app.
hs_authorize() {
  local user="$1" url="$2" subject="${3:-}" ehr="${4:-}" location page action form_token
  location="$(hs_redirect "$url")"
  case "$HS_UPSTREAM" in
    zitadel)
      # NOTE: ZITADEL's login-ui/oidc-standard.mdx, the custom login flow: a session
      # with a password check, then the callback of the auth request it finishes.
      location="$(hs_redirect "$location")"
      local request session id session_token pat
      request="$(sed -nE 's/.*[?&]authRequest=([^&]+).*/\1/p' <<<"$location")"
      pat="$(cat "$(hs_dir)/zitadel/login-client.pat")"
      session="$(curl -s -X POST -H "Authorization: Bearer $pat" -H 'Content-Type: application/json' \
        "$HS_ZITADEL/zitadel.session.v2.SessionService/CreateSession" \
        -d "{\"checks\":{\"user\":{\"loginName\":\"$user\"},\"password\":{\"password\":\"$(hs_password "$user")\"}}}")"
      id="$(jq -r '.sessionId // empty' <<<"$session")"
      session_token="$(jq -r '.sessionToken // empty' <<<"$session")"
      location="$(curl -s -X POST -H "Authorization: Bearer $pat" -H 'Content-Type: application/json' \
        "$HS_ZITADEL/zitadel.oidc.v2.OIDCService/CreateCallback" \
        -d "{\"authRequestId\":\"$request\",\"session\":{\"sessionId\":\"$id\",\"sessionToken\":\"$session_token\"}}" |
        jq -r '.callbackUrl // empty')" ;;
    *)
      page="$(curl -sS -c "$HS_JAR" -b "$HS_JAR" "$location")"
      action="$(hs_attribute "$page" '<form[^>]*id="kc-form-login"[^>]*>' action)"
      location="$(hs_redirect --data-urlencode "username=$user" \
        --data-urlencode "password=$(hs_password "$user")" "$action")" ;;
  esac
  case "$location" in "$HS_ISSUER"/callback*) ;; *)
    echo "      the login of $user did not return to FerroSMART: ${location:0:200}" >&2; return 1 ;; esac
  location="$(hs_redirect "$location")"
  if [[ "$location" == "$HS_ISSUER"/picker* ]]; then
    page="$(curl -sS -c "$HS_JAR" -b "$HS_JAR" -G "$HS_ISSUER/picker" --data-urlencode "subject_id=$subject")"
    printf '%s' "$page" > "$HS_JAR.picker"
    form_token="$(hs_attribute "$page" '<input[^>]*name="form_token"[^>]*>' value)"
    location="$(hs_redirect --data-urlencode "form_token=$form_token" --data-urlencode "subject_id=$subject" \
      --data-urlencode "ehr_id=$ehr" "$HS_ISSUER/picker")"
  fi
  case "$location" in "$HS_ISSUER"/consent*) ;; *)
    echo "      FerroSMART did not ask $user for consent (picker subject '$subject', EHR '$ehr'): ${location:0:200}" >&2
    hs_answered; return 1 ;; esac
  page="$(curl -sS -c "$HS_JAR" -b "$HS_JAR" "$location")"
  form_token="$(hs_attribute "$page" '<input[^>]*name="form_token"[^>]*>' value)"
  local -a approve=(--data-urlencode "form_token=$form_token" --data-urlencode "decision=approve")
  local offered
  while IFS= read -r offered; do
    [[ -n "$offered" ]] && approve+=(--data-urlencode "scope=$offered")
  done < <(grep -oE '<input[^>]*name="scope"[^>]*>' <<<"$page" | grep -oE 'value="[^"]*"' | sed -E 's/^value="//; s/"$//')
  hs_redirect "${approve[@]}" "$HS_ISSUER/consent"
}

# The token response of `user`'s authorization code flow through the app for
# `scope`, with the EHR launch `launch` or the picker's `subject` and `ehr`.
hs_code_flow() {
  local user="$1" scope="$2" launch="${3:-}" subject="${4:-}" ehr="${5:-}"
  local verifier challenge state url location code
  HS_JAR="$(mktemp "$PROBE_TMP/jar.XXXXXX")"
  verifier="$(openssl rand -hex 32)"
  challenge="$(printf '%s' "$verifier" | openssl dgst -sha256 -binary | hs_b64url)"
  state="$(openssl rand -hex 16)"
  url="$HS_ISSUER/authorize?response_type=code&client_id=$HS_APP&redirect_uri=$(jq -rn --arg v "$HS_REDIRECT" '$v|@uri')"
  url+="&scope=$(jq -rn --arg v "$scope" '$v|@uri')&state=$state&aud=$(jq -rn --arg v "$HS_BASE" '$v|@uri')"
  url+="&code_challenge=$challenge&code_challenge_method=S256"
  [[ -n "$launch" ]] && url+="&launch=$(jq -rn --arg v "$launch" '$v|@uri')"
  location="$(hs_authorize "$user" "$url" "$subject" "$ehr")" || { rm -f "$HS_JAR" "$HS_JAR".*; return 1; }
  rm -f "$HS_JAR" "$HS_JAR".*
  [[ "$(hs_param state "$location")" = "$state" ]] || { echo "      the state came back changed: $location" >&2; return 1; }
  code="$(hs_param code "$location")"
  [[ -n "$code" ]] || { echo "      no code for $user: $(hs_param error "$location")" >&2; return 1; }
  curl -s "$HS_ISSUER/token" --data-urlencode grant_type=authorization_code --data-urlencode "code=$code" \
    --data-urlencode "redirect_uri=$HS_REDIRECT" --data-urlencode "client_id=$HS_APP" \
    --data-urlencode "code_verifier=$verifier"
}

# A professional's token through the app.
hs_person_token() {
  local user="$1" answer
  hs_cached "person-$user" && return 0
  answer="$(hs_code_flow "$user" "$HS_USER_SCOPE")" || return 1
  hs_keep "person-$user" "$answer"
}

# A patient's token through the app: a standalone launch with
# `launch/patient`, the picker choosing the EHR `ehr`.
hs_patient_token() {
  local user="$1" ehr="$2" subject answer
  hs_cached "patient-$user-$ehr" && return 0
  subject="$(cat "$(hs_dir)/subject-$ehr" 2>/dev/null)"
  answer="$(hs_code_flow "$user" "$HS_PATIENT_SCOPE" "" "$subject" "$ehr")" || return 1
  hs_keep "patient-$user-$ehr" "$answer"
}

# Notes that the EHR `ehr` names `subject` as its subject, for the picker.
hs_subject() { printf '%s' "$2" > "$(hs_dir)/subject-$1"; }

# A login identifier element of FerroSMART's subject `sub`, for a PERSON's details.
j_login() {
  jq -nc --arg i "$HS_LOGIN_ISSUER" --arg s "$1" '{details:{_type:"ITEM_TREE",archetype_node_id:"at0010",
    name:{_type:"DV_TEXT",value:"details"},items:[{_type:"ELEMENT",archetype_node_id:"at0011",
    name:{_type:"DV_TEXT",value:"login"},value:{_type:"DV_IDENTIFIER",issuer:$i,id:$s,type:"login"}}]}}'
}
