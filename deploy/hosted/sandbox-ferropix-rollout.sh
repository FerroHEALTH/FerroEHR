#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Brings FerroPIX and FerroTERM 0.3 onto the hosted sandbox (#4242): every
# operator step on the two boxes, run from a checkout on the operator's own
# machine over SSH as the `deploy` user. Each step prints `==> step N: …` and
# then `OK:`, `SKIP:` or `FAIL:` lines with their evidence, so a pasted log
# shows where a run stopped. Every step checks before it changes anything, so a
# re-run is safe, and `--check` changes nothing at all.
#
#   deploy/hosted/sandbox-ferropix-rollout.sh            # steps 1 to 6
#   deploy/hosted/sandbox-ferropix-rollout.sh --check    # verify only
#   deploy/hosted/sandbox-ferropix-rollout.sh --step 2   # one step (repeatable)
#   deploy/hosted/sandbox-ferropix-rollout.sh --self-test  # prove --check pulls no image
#
# Steps: 1 preflight; 2 the FerroPIX database and role on the DB box; 3 the
# app box's `.env` lines and its deploy script; 4 the FerroTERM index layout;
# 5 after the v5.5.0 deploy: recreate, seed FerroPIX and verify end to end;
# 6 the DB box onto the pinned ferroehr-postgres image (#4252).
#
# It runs at the release the sandbox serves (#4372): before any step it reads
# `solution_version` from the ITS-REST system OPTIONS and refuses, naming the
# tag to check out, when this checkout's workspace version is another one, so
# the routes it probes and the deploy script it installs are that release's.
#
# A secret is generated on the box it belongs to, or read from the
# environment (FERROPIX_DB_PASSWORD, FERROPIX_DATA_KEY), and is never printed:
# the log shows `set` or the first 12 hex digits of its SHA-256.
#
# No specification governs this rollout: our own design.
set -euo pipefail

APP_HOST="${APP_HOST:-167.233.172.220}"
DB_HOST="${DB_HOST:-postgres.ferroehr.eu}"
SSH_USER="${SSH_USER:-deploy}"
PUBLIC_URL="${PUBLIC_URL:-https://sandbox.ferroehr.eu}"
APP_DIR="${APP_DIR:-/opt/ferroehr-sandbox}"
DB_DIR="${DB_DIR:-/opt/ferroehr-postgres}"
DB_CONTAINER="${DB_CONTAINER:-ferroehr-sandbox-postgres}"
# The port the app box dials the DB box on, and the network the DB box must
# publish PostgreSQL on alone (the Hetzner private network).
DB_PORT="${DB_PORT:-5432}"
PRIVATE_PREFIX="${PRIVATE_PREFIX:-10.0.0.}"
# The address the app box dials; empty for the DB box's PG_BIND_IP.
DB_DIAL_HOST="${DB_DIAL_HOST:-}"
# The first FerroEHR release whose sandbox posture runs FerroPIX.
RELEASE_FLOOR="${RELEASE_FLOOR:-5.5.0}"
# The ssh client; a stand-in box replaces it for a local rehearsal.
SSH_CMD="${ROLLOUT_SSH:-ssh}"
# The demo credential the public sandbox publishes (ferroehr.sandbox.toml).
DEMO_AUTH="${SANDBOX_USER:-ferroehr}:${SANDBOX_PASS:-ferroehr}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/scripts/sandbox/seed/manifest.json"
CLOUD_INIT="$ROOT/deploy/hosted/cloud-init.yaml"
CLOUD_INIT_DB="$ROOT/deploy/hosted/cloud-init-postgres.yaml"
INDEX_RECORD="$ROOT/deploy/hosted/ferroterm-index.env"
SOURCE_DOMAIN="urn:oid:2.999.1.1"
# The image the DB box runs: the default cloud-init-postgres.yaml pins.
DB_IMAGE="$(sed -nE 's/.*\$\{FERROEHR_POSTGRES_IMAGE:-([^}]+)\}.*/\1/p' "$CLOUD_INIT_DB" | sed -n 1p)"
# The FerroTERM the posture pins, and its version: the default the compose
# file gives FERROTERM_IMAGE.
FERROTERM_PIN="$(sed -nE 's/.*\$\{FERROTERM_IMAGE:-([^}]+)\}.*/\1/p' "$ROOT/deploy/hosted/docker-compose.yml" | sed -n 1p)"
FERROTERM_PIN_VERSION="$(sed -nE 's#.*:([0-9][0-9.]*)@sha256:.*#\1#p' <<<"$FERROTERM_PIN")"
# The FerroPIX the posture pins, and its version: the default the compose file
# gives FERROPIX_IMAGE (#4392).
FERROPIX_PIN="$(sed -nE 's/.*\$\{FERROPIX_IMAGE:-([^}]+)\}.*/\1/p' "$ROOT/deploy/hosted/docker-compose.yml" | sed -n 1p)"
FERROPIX_PIN_VERSION="$(sed -nE 's#.*:([0-9][0-9.]*)@sha256:.*#\1#p' <<<"$FERROPIX_PIN")"

CHECK=0
SELF_TEST=0
ASSUME_RELEASE=0
STEPS=()
usage() {
  echo "usage: $0 [--check] [--step N]... [--assume-release] | --self-test" >&2
  echo "  --assume-release  treat the running CDR as $RELEASE_FLOOR+ (a stand-in box on an unreleased image)" >&2
}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK=1 ;;
    --step)
      [[ "${2:-}" =~ ^[1-6]$ ]] || { usage; exit 2; }
      STEPS+=("$2")
      shift
      ;;
    --assume-release) ASSUME_RELEASE=1 ;;
    --self-test) SELF_TEST=1 ;;
    -h | --help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
  shift
done
[[ ${#STEPS[@]} -gt 0 ]] || STEPS=(1 2 3 4 5 6)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

step() { printf '\n==> step %s: %s\n' "$1" "$2"; }
ok() { printf 'OK: %s\n' "$*"; }
skip() { printf 'SKIP: %s\n' "$*"; }
fail() {
  printf 'FAIL: %s\n' "$*"
  exit 1
}
fingerprint() {
  local secret="$1"
  printf '%s' "$secret" | shasum -a 256 | cut -c1-12
}
want_step() {
  local want="$1" s
  for s in "${STEPS[@]}"; do [[ "$s" == "$want" ]] && return 0; done
  return 1
}
# Indents a box's output under its name in the log.
indent_app() { sed 's/^/  app: /'; }
# The curl --write-out format that prints only the HTTP status.
readonly HTTP_STATUS='%{http_code}'

# Runs the script on stdin on a box. The script travels on stdin into a file
# only its user reads, so a secret embedded in it never appears on a command
# line, and no command in it can read the rest of the script as its input.
remote() {
  local host="$1"
  # shellcheck disable=SC2016 # the command is expanded by the box's shell, not this one
  "$SSH_CMD" -o BatchMode=yes -o ConnectTimeout=15 "$SSH_USER@$host" \
    'f="$(mktemp)"; cat > "$f"; bash "$f" < /dev/null; rc=$?; rm -f "$f"; exit "$rc"'
}
app() { remote "$APP_HOST"; }
db() { remote "$DB_HOST"; }

# 0 when version $1 is a release at or past $2.
version_at_least() {
  local version="$1" floor="$2"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  [[ "$(printf '%s\n%s\n' "$floor" "$version" | sort -t. -k1,1n -k2,2n -k3,3n | sed -n 1p)" == "$floor" ]]
}

running_cdr_version() {
  app <<'EOF' || true
docker inspect --format '{{index .Config.Labels "org.opencontainers.image.version"}}' ferroehr-sandbox 2>/dev/null
EOF
}

# The FerroEHR version this checkout builds: the workspace version.
checkout_version() {
  sed -nE '/^\[workspace\.package\]/,/^\[/ s/^version = "([^"]+)"/\1/p' "$ROOT/Cargo.toml" | sed -n 1p
}

# The version the sandbox serves: `solution_version` on the ITS-REST 1.1.0
# system OPTIONS (specifications/schemas/others/Options.yaml), the one route
# no FerroEHR release moves.
served_version() {
  curl -sS --fail -m 30 -X OPTIONS "$PUBLIC_URL/ferroehr/rest/openehr/v1" | jq -r '.solution_version // empty'
}

# Judges a served version $1 against this checkout's $2: prints the verdict
# and returns 0 only when they are the same release.
release_verdict() {
  local served="$1" checkout="$2"
  if ! [[ "$served" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "the sandbox's OPTIONS answered solution_version '${served}', which names no FerroEHR release"
    return 1
  fi
  if [[ "$served" != "$checkout" ]]; then
    echo "the sandbox serves FerroEHR $served and this checkout builds ${checkout:-nothing}; run this script from the tag v$served (git worktree add ../ferroehr-v$served v$served)"
    return 1
  fi
  echo "the sandbox serves FerroEHR $served, the release this checkout builds"
}

release_knows_ferropix() {
  local version="$1"
  [[ "$ASSUME_RELEASE" -eq 1 ]] && return 0
  version_at_least "$version" "$RELEASE_FLOOR"
}

# What ferropix_login prints when --check finds no psql client image to run.
readonly NO_CLIENT_IMAGE="no-psql-client-image"

# Logs in as ferropix from the app box over the private network ($1 the
# password, $2 the DB box's private address) and prints the server's answer.
# The pinned image is the psql client, and `docker run` would pull it, so
# under --check the login runs only when the image is already on the box.
ferropix_login() {
  app > "$WORK/login" <<EOF || true
if [ $CHECK -eq 1 ] && ! docker image inspect $(printf '%q' "$DB_IMAGE") > /dev/null 2>&1; then
  echo $NO_CLIENT_IMAGE
  exit 0
fi
printf '%s' $(printf '%q' "$1") | docker run -i --rm --entrypoint sh $(printf '%q' "$DB_IMAGE") -c \
  'PGPASSWORD="\$(cat)" psql -h ${DB_DIAL_HOST:-$2} -p $DB_PORT -U ferropix -d ferropix -At -F " " -c "SELECT current_user, current_database(), rolsuper FROM pg_roles WHERE rolname = current_user"'
EOF
  cat "$WORK/login"
}

# Judges a ferropix_login answer $1: SKIP when --check had no client image,
# FAIL unless the server answered as ferropix, else OK with the answer.
judge_login() {
  local out="$1" bind="$2" note="${3-}"
  if [[ "$out" == "$NO_CLIENT_IMAGE" ]]; then
    skip "the psql client image is not on the app box; the real run checks the login"
    return
  fi
  [[ "$out" == "ferropix ferropix f" ]] \
    || fail "the app box cannot log in as ferropix at ${DB_DIAL_HOST:-$bind}:$DB_PORT: $out"
  ok "from the app box over the private network: $out$note"
}

# Runs ferropix_login against a stub box whose docker records each call:
# --check with the image absent pulls nothing, and every other case logs in.
self_test() {
  local stub="$WORK/stub" failures=0 check present want out ran
  mkdir -p "$stub"
  cat > "$stub/ssh" <<'STUB'
#!/usr/bin/env bash
eval "${*: -1}"
STUB
  cat > "$stub/docker" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$DOCKER_LOG"
case "$1" in
  image) [ "$STUB_IMAGE_PRESENT" -eq 1 ] ;;
  run) cat > /dev/null; echo "ferropix ferropix f" ;;
  *) exit 1 ;;
esac
STUB
  chmod +x "$stub/ssh" "$stub/docker"
  SSH_CMD="$stub/ssh"
  export PATH="$stub:$PATH" DOCKER_LOG="$WORK/docker.log"
  while read -r check present want; do
    : > "$DOCKER_LOG"
    CHECK="$check"
    out="$(STUB_IMAGE_PRESENT="$present" ferropix_login pw 10.0.0.2)"
    ran=no
    grep -q '^run ' "$DOCKER_LOG" && ran=yes
    if [[ "$want" == skip && ( "$out" != "$NO_CLIENT_IMAGE" || "$ran" == yes ) ]] \
      || [[ "$want" == login && ( "$out" != "ferropix ferropix f" || "$ran" == no ) ]]; then
      echo "self-test: check=$check image=$present: got '$out' (docker run: $ran), want $want" >&2
      failures=1
    fi
  done <<'CASES'
1 0 skip
1 1 login
0 0 login
0 1 login
CASES
  local served checkout verdict
  while read -r served checkout want; do
    verdict=refuse
    release_verdict "$served" "$checkout" > /dev/null && verdict=run
    if [[ "$verdict" != "$want" ]]; then
      echo "self-test: served=$served checkout=$checkout: got $verdict, want $want" >&2
      failures=1
    fi
  done <<'CASES'
5.5.0 5.5.0 run
5.6.0-rc3 5.6.0-rc3 run
5.5.0 5.6.0-rc2 refuse
5.6.0-rc2 5.6.0-rc3 refuse
- 5.5.0 refuse
CASES
  [[ "$failures" -eq 0 ]] || return 1
  echo "sandbox-ferropix-rollout: self-test OK (--check runs no docker run without the client image; 3 login cases run it; only the served release runs)."
}

# ── step 1: preflight ─────────────────────────────────────────────────────────
step1() {
  step 1 "preflight: both boxes answer, and what they run now"
  local out
  app > "$WORK/out" <<EOF || fail "cannot reach the app box $SSH_USER@$APP_HOST over SSH"
cd "$APP_DIR"
echo "host \$(hostname)"
for c in ferroehr-sandbox ferroehr-sandbox-viewer ferroehr-sandbox-ferroterm \\
  ferroehr-sandbox-ferropix ferroehr-sandbox-issuer ferroehr-sandbox-caddy; do
  # A missing container still makes docker inspect print an empty line.
  s="\$(docker inspect --format "\$c {{.Config.Image}} version={{index .Config.Labels \"org.opencontainers.image.version\"}} {{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}" "\$c" 2>/dev/null | sed '/^\$/d')" || true
  echo "\${s:-\$c absent}"
done
docker compose version --short | sed 's/^/compose /'
EOF
  out="$(cat "$WORK/out")"
  printf '%s\n' "$out" | indent_app
  ok "app box $APP_HOST answers"
  db > "$WORK/out" <<EOF || fail "cannot reach the DB box $SSH_USER@$DB_HOST, or $DB_CONTAINER is not running"
echo "host \$(hostname)"
docker inspect --format '$DB_CONTAINER {{.Config.Image}} {{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "$DB_CONTAINER"
docker exec "$DB_CONTAINER" psql -U postgres -Atc 'SHOW server_version'
EOF
  out="$(cat "$WORK/out")"
  printf '%s\n' "$out" | sed 's/^/  db: /'
  ok "DB box $DB_HOST answers"
  ok "this checkout is $(git -C "$ROOT" rev-parse --short HEAD) on $(git -C "$ROOT" rev-parse --abbrev-ref HEAD)"
}

# ── step 2: the FerroPIX database and role on the DB box ──────────────────────
step2() {
  step 2 "DB box: the ferropix database and role, reachable over the private network only"
  local given="${FERROPIX_DB_PASSWORD:-}" out
  [[ "$given" =~ ^[A-Za-z0-9]*$ ]] || fail "FERROPIX_DB_PASSWORD may hold letters and digits only (it goes into a URL)"
  db > "$WORK/out" <<EOF || fail "the DB box refused the step; its output is above"
set -euo pipefail
cd "$DB_DIR"
psql() { docker exec -i "$DB_CONTAINER" psql -U postgres -v ON_ERROR_STOP=1 -Atq "\$@"; }
role="\$(psql -c "SELECT rolsuper FROM pg_roles WHERE rolname = 'ferropix'")"
dbx="\$(psql -c "SELECT 1 FROM pg_database WHERE datname = 'ferropix'")"
stored="\$(sed -n 's/^FERROPIX_DB_PASSWORD=//p' .env | tail -n 1)"
echo "role=\${role:-absent} database=\${dbx:-absent} stored_password=\$([ -n "\$stored" ] && echo set || echo absent)"
if [ "$CHECK" -eq 1 ]; then
  [ "\$role" = f ] && [ "\$dbx" = 1 ] && [ -n "\$stored" ] || { echo "verdict=missing"; exit 0; }
else
  pw=$(printf '%q' "$given")
  [ -n "\$pw" ] || pw="\$stored"
  fresh=0
  if [ -z "\$pw" ]; then
    pw="\$(openssl rand -hex 24)"
    fresh=1
  fi
  if [ "\$pw" != "\$stored" ]; then
    umask 077
    { grep -v '^FERROPIX_DB_PASSWORD=' .env || true; echo "FERROPIX_DB_PASSWORD=\$pw"; } > .env.next
    mv .env.next .env
    chmod 600 .env
    echo "wrote FERROPIX_DB_PASSWORD to $DB_DIR/.env (fresh=\$fresh)"
  fi
  if [ -z "\$role" ]; then
    printf "CREATE ROLE ferropix LOGIN PASSWORD '%s';\n" "\$pw" | psql
    echo "created role ferropix"
  elif [ "\$pw" != "\$stored" ]; then
    printf "ALTER ROLE ferropix PASSWORD '%s';\n" "\$pw" | psql
    echo "set the password of role ferropix"
  fi
  if [ -z "\$dbx" ]; then
    psql -c 'CREATE DATABASE ferropix'
    echo "created database ferropix"
  fi
  psql -c 'REVOKE ALL ON DATABASE ferropix FROM PUBLIC' \\
    -c 'GRANT CONNECT, CREATE, TEMPORARY ON DATABASE ferropix TO ferropix'
  role="\$(psql -c "SELECT rolsuper FROM pg_roles WHERE rolname = 'ferropix'")"
fi
[ "\$role" = f ] || { echo "verdict=superuser"; exit 0; }
echo "grants=\$(psql -c "SELECT datacl FROM pg_database WHERE datname = 'ferropix'")"
echo "published=\$(docker port "$DB_CONTAINER" 5432/tcp | tr '\n' ' ')"
echo "bind_ip=\$(sed -n 's/^PG_BIND_IP=//p' .env | tail -n 1)"
echo "verdict=ready"
EOF
  out="$(cat "$WORK/out")"
  printf '%s\n' "$out" | sed 's/^/  db: /'
  case "$(sed -n 's/^verdict=//p' <<<"$out")" in
    ready) ;;
    missing) fail "the ferropix role, database or stored password is missing; run this step without --check" ;;
    superuser) fail "the ferropix role is a superuser, which FerroPIX refuses at boot" ;;
    *) fail "the DB box gave no verdict" ;;
  esac
  ok "role ferropix (no superuser) and database ferropix exist; PUBLIC holds no grant on it"
  local published bind
  published="$(sed -n 's/^published=//p' <<<"$out")"
  bind="$(sed -n 's/^bind_ip=//p' <<<"$out")"
  [[ -n "$bind" && "$bind" == "$PRIVATE_PREFIX"* ]] \
    || fail "PG_BIND_IP ($bind) is not on the private network $PRIVATE_PREFIX*"
  # Every published address must be the private one: no 0.0.0.0 and no [::].
  local addr
  for addr in $published; do
    [[ "$addr" == "$bind:"* ]] || fail "PostgreSQL is also published on $addr, outside the private network"
  done
  ok "PostgreSQL is published on $published alone (the private network)"

  # The proof from the side that matters: the app box logs in as ferropix
  # over the private network.
  local pw
  db > "$WORK/out" <<EOF
sed -n 's/^FERROPIX_DB_PASSWORD=//p' "$DB_DIR/.env" | tail -n 1
EOF
  pw="$(cat "$WORK/out")"
  [[ -n "$pw" ]] || fail "no FERROPIX_DB_PASSWORD on the DB box"
  out="$(ferropix_login "$pw" "$bind")"
  judge_login "$out" "$bind" " (password fingerprint $(fingerprint "$pw"))"
  printf '%s' "$pw" > "$WORK/db-password"
}

# ── step 3: the app box's .env lines and deploy script ────────────────────────
step3() {
  step 3 "app box: the .env lines and the deploy script"
  local version known=0 reason pw="" bind=""
  version="$(running_cdr_version)"
  # The lesson the README records: the CDR's strict sweep refuses a FERROEHR_
  # name its release does not know. A line is written active once the running
  # release is past the floor, or when no container of the installed posture
  # receives it (a compose substitution name), so it cannot stop that release.
  if release_knows_ferropix "$version"; then
    known=1
    reason="the running release ($version) knows them"
  else
    reason="the running release (${version:-none}) predates $RELEASE_FLOOR"
  fi

  if [[ "$CHECK" -eq 0 ]]; then
    if [[ -s "$WORK/db-password" ]]; then
      pw="$(cat "$WORK/db-password")"
    else
      db > "$WORK/out" <<EOF
sed -n 's/^FERROPIX_DB_PASSWORD=//p' "$DB_DIR/.env" | tail -n 1
EOF
      pw="$(cat "$WORK/out")"
    fi
    [[ -n "$pw" ]] || fail "no FERROPIX_DB_PASSWORD on the DB box; run step 2 first"
    db > "$WORK/out" <<EOF
sed -n 's/^PG_BIND_IP=//p' "$DB_DIR/.env" | tail -n 1
EOF
    bind="$(cat "$WORK/out")"
  fi

  local out
  app > "$WORK/out" <<EOF || fail "the app box refused the step"
set -euo pipefail
cd "$APP_DIR"
installed_passes() {
  # The names a container of the installed posture receives in its environment.
  docker compose config --format json 2>/dev/null \\
    | grep -oE '"(FERROPIX_DATABASE_URL|FERROPIX_DATA_KEY|FERROTERM_INDEXES)"' | sort -u | tr '\n' ' '
}
passed="\$(installed_passes || true)"
active=$known
if [ "\$active" -eq 0 ] && [ -z "\$passed" ]; then
  active=1
  echo "reason=no container of the installed posture receives these names (compose substitution only)"
fi
echo "state_before=\$(grep -cE '^#? ?(FERROPIX_DATABASE_URL|FERROPIX_DATA_KEY|FERROTERM_INDEXES)=' .env || true) lines"
for name in FERROPIX_DATABASE_URL FERROPIX_DATA_KEY FERROTERM_INDEXES; do
  if grep -qE "^\$name=" .env; then echo "\$name=active"
  elif grep -qE "^# ?\$name=" .env; then echo "\$name=commented"
  else echo "\$name=absent"; fi
done
legacy="\$(sed -n 's/^FERROTERM_INDEX=//p' .env | tail -n 1)"
echo "ferroterm_index_legacy=\$([ -n "\$legacy" ] && echo "\$legacy" || echo unset)"
# A FERROTERM_IMAGE line overrides the FerroTERM the posture pins. The box
# carried FerroTERM 0.1.6 that way, which reads none of the 0.3 names the
# compose file passes, so it opened no index and served no LOINC (#4278).
override="\$(sed -n 's/^FERROTERM_IMAGE=//p' .env | tail -n 1)"
if [ -z "\$override" ]; then echo "ferroterm_image=unset"
elif [ "\$override" = "$FERROTERM_PIN" ]; then echo "ferroterm_image=pin"
else echo "ferroterm_image=stale \$override"; fi
# A FERROPIX_IMAGE line overrides the FerroPIX the posture pins, whose
# configuration the posture writes for that release alone (#4392).
pix_override="\$(sed -n 's/^FERROPIX_IMAGE=//p' .env | tail -n 1)"
if [ "$CHECK" -eq 0 ]; then
  umask 077
  cp -p .env .env.prev
  key="\$(sed -nE 's/^#? ?FERROPIX_DATA_KEY=//p' .env | tail -n 1)"
  [ -n "\$key" ] || key=$(printf '%q' "${FERROPIX_DATA_KEY:-}")
  [ -n "\$key" ] || key="\$(openssl rand -base64 32)"
  dsn="postgres://ferropix:$(printf '%q' "${pw:-}")@${DB_DIAL_HOST:-$bind}:$DB_PORT/ferropix"
  indexes="\$(printf '%s' "\$legacy" | tr ':' ',')"
  prefix=""
  [ "\$active" -eq 1 ] || prefix="# "
  {
    grep -vE '^#? ?(FERROPIX_DATABASE_URL|FERROPIX_DATA_KEY|FERROTERM_INDEXES)=|^# FerroPIX and FerroTERM 0.3 \(#4242\)' .env || true
    echo "# FerroPIX and FerroTERM 0.3 (#4242), written by sandbox-ferropix-rollout.sh"
    echo "\${prefix}FERROPIX_DATABASE_URL=\$dsn"
    echo "\${prefix}FERROPIX_DATA_KEY=\$key"
    [ -z "\$indexes" ] || echo "\${prefix}FERROTERM_INDEXES=\$indexes"
  } > .env.next
  mv .env.next .env
  if [ -n "\$override" ] && [ "\$override" != "$FERROTERM_PIN" ]; then
    sed -i -E 's|^(FERROTERM_IMAGE=.*)\$|# \1 (retired by sandbox-ferropix-rollout.sh, #4278: the posture pins FerroTERM)|' .env
  fi
  if [ -n "\$pix_override" ] && [ "\$pix_override" != "$FERROPIX_PIN" ]; then
    sed -i -E 's|^(FERROPIX_IMAGE=.*)\$|# \1 (retired by sandbox-ferropix-rollout.sh, #4392: the posture pins FerroPIX)|' .env
  fi
  chmod 600 .env
  cmp -s .env .env.prev && echo "changed=no" || echo "changed=yes (previous: $APP_DIR/.env.prev)"
fi
for name in FERROPIX_DATABASE_URL FERROPIX_DATA_KEY FERROTERM_INDEXES; do
  v="\$(sed -nE "s/^#? ?\$name=//p" .env | tail -n 1)"
  state=absent
  grep -qE "^\$name=" .env && state=active
  grep -qE "^# ?\$name=" .env && state=commented
  echo "after \$name \$state \$([ -n "\$v" ] && printf '%s' "\$v" | sha256sum | cut -c1-12)"
done
echo "after_ferroterm_image=\$(grep -qE '^FERROTERM_IMAGE=' .env && sed -n 's/^FERROTERM_IMAGE=//p' .env | tail -n 1 || echo unset)"
echo "after_ferropix_image=\$(grep -qE '^FERROPIX_IMAGE=' .env && sed -n 's/^FERROPIX_IMAGE=//p' .env | tail -n 1 || echo unset)"
echo "env_mode=\$(stat -c %a .env)"
EOF
  out="$(cat "$WORK/out")"
  # Only names, states and fingerprints come back; no value does.
  printf '%s\n' "$out" | indent_app
  local r
  r="$(sed -n 's/^reason=//p' <<<"$out")"
  ok "${r:-$reason}"
  if [[ "$CHECK" -eq 1 ]]; then
    grep -qE '^after FERROPIX_DATABASE_URL (active|commented) ' <<<"$out" \
      || fail "the .env carries no FERROPIX_DATABASE_URL line; run step 3 without --check"
    grep -qE '^after FERROPIX_DATA_KEY (active|commented) ' <<<"$out" \
      || fail "the .env carries no FERROPIX_DATA_KEY line; run step 3 without --check"
  fi
  [[ "$(sed -n 's/^env_mode=//p' <<<"$out")" == 600 ]] || fail ".env is not mode 600"
  ok "the .env lines are in place (states and fingerprints above)"
  local image_after
  image_after="$(sed -n 's/^after_ferroterm_image=//p' <<<"$out")"
  if [[ "$image_after" == unset || "$image_after" == "$FERROTERM_PIN" ]]; then
    ok "no FERROTERM_IMAGE line overrides the pinned FerroTERM $FERROTERM_PIN_VERSION"
  else
    fail "the .env pins FerroTERM $image_after over the posture's $FERROTERM_PIN; FerroTERM 0.1 reads none of the 0.3 names, so it serves no index and no LOINC; run step 3 without --check"
  fi
  image_after="$(sed -n 's/^after_ferropix_image=//p' <<<"$out")"
  if [[ "$image_after" == unset || "$image_after" == "$FERROPIX_PIN" ]]; then
    ok "no FERROPIX_IMAGE line overrides the pinned FerroPIX $FERROPIX_PIN_VERSION"
  else
    fail "the .env pins FerroPIX $image_after over the posture's $FERROPIX_PIN, whose ferropix.sandbox.toml is written for $FERROPIX_PIN_VERSION; run step 3 without --check"
  fi

  # The deploy script does not update itself, and the reseed's `identities`
  # verb needs this version of it.
  awk -v want="  - path: /opt/ferroehr-sandbox/deploy.sh" '
    $0 == want { entry = 1; next }
    entry && /^[[:space:]]*content: \|/ { body = 1; next }
    body {
      if ($0 == "") { print ""; next }
      if (indent == 0) { match($0, /^[[:space:]]*/); indent = RLENGTH }
      if (match($0, /^[[:space:]]*/) && RLENGTH < indent) { exit }
      print substr($0, indent + 1); next
    }
  ' "$CLOUD_INIT" > "$WORK/deploy.sh"
  [[ -s "$WORK/deploy.sh" ]] || fail "could not extract deploy.sh from $CLOUD_INIT"
  local want have
  want="$(shasum -a 256 < "$WORK/deploy.sh" | cut -c1-12)"
  app > "$WORK/out" <<EOF || true
sha256sum "$APP_DIR/deploy.sh" | cut -c1-12
EOF
  have="$(cat "$WORK/out")"
  if [[ "$have" == "$want" ]]; then
    ok "deploy.sh on the box is this checkout's ($want)"
  elif [[ "$CHECK" -eq 1 ]]; then
    fail "deploy.sh on the box ($have) is not this checkout's ($want); run step 3 without --check"
  else
    app <<EOF > /dev/null
set -euo pipefail
cd "$APP_DIR"
[ ! -f deploy.sh ] || cp -p deploy.sh deploy.sh.prev
base64 -d > deploy.sh.next <<'B64'
$(base64 < "$WORK/deploy.sh")
B64
chmod 755 deploy.sh.next
mv deploy.sh.next deploy.sh
EOF
    app > "$WORK/out" <<EOF
sha256sum "$APP_DIR/deploy.sh" | cut -c1-12
EOF
    have="$(cat "$WORK/out")"
    [[ "$have" == "$want" ]] || fail "installed deploy.sh reads $have, wanted $want"
    ok "installed deploy.sh $want (previous: $APP_DIR/deploy.sh.prev)"
  fi
}

# ── step 4: the FerroTERM index ───────────────────────────────────────────────
step4() {
  step 4 "the FerroTERM index: does the pinned FerroTERM read the index the box carries"
  local want out
  want="$(sed -n 's/^FERROTERM_INDEX_LAYOUT=//p' "$INDEX_RECORD" | tail -n 1)"
  [[ -n "$want" ]] || fail "$INDEX_RECORD names no FERROTERM_INDEX_LAYOUT"
  app > "$WORK/out" <<EOF || fail "cannot read the index directory on the app box"
cd "$APP_DIR"
dir="\$(sed -n 's/^FERROTERM_INDEX_DIR=//p' .env | tail -n 1)"
dir="\${dir:-$APP_DIR/ferroterm-index}"
found=0
for m in "\$dir"/*/manifest.json; do
  [ -f "\$m" ] || continue
  found=1
  echo "\$m \$(grep -oE '"storeLayout" *: *"?[0-9]+' "\$m" | grep -oE '[0-9]+\$')"
done
[ "\$found" -eq 1 ] || echo "none"
EOF
  out="$(cat "$WORK/out")"
  printf '%s\n' "$out" | indent_app
  if [[ "$out" == none ]]; then
    skip "the box carries no index; FerroTERM serves the shaped seed alone"
    return
  fi
  if awk -v w="$want" '$2 != w { bad = 1 } END { exit bad }' <<<"$out"; then
    ok "every index is layout $want, the layout the pinned FerroTERM reads: no rebuild"
    return
  fi
  # A rebuild: the archives are licensed and stay on this machine; only the
  # built index travels to the box.
  [[ "$CHECK" -eq 0 ]] || fail "the box's index is not layout $want; run step 4 without --check with SNOMED_RF2_ZIP and LOINC_ZIP set"
  local image
  image="$(sed -nE 's#.*(ghcr\.io/ferrohealth/ferroterm:[0-9.]+@sha256:[0-9a-f]+).*#\1#p' "$ROOT/deploy/hosted/docker-compose.yml" | sed -n 1p)"
  [[ -n "${SNOMED_RF2_ZIP:-}" && -f "${SNOMED_RF2_ZIP:-}" && -n "${LOINC_ZIP:-}" && -f "${LOINC_ZIP:-}" ]] \
    || fail "set SNOMED_RF2_ZIP and LOINC_ZIP to the licensed release archives on this machine and re-run step 4"
  local out_dir="$WORK/index"
  mkdir -p "$out_dir"
  docker run --rm --user "$(id -u):$(id -g)" -v "$(dirname "$SNOMED_RF2_ZIP"):/in:ro" -v "$out_dir:/out" \
    --entrypoint /usr/local/bin/ferroterm-build "$image" --rf2 "/in/$(basename "$SNOMED_RF2_ZIP")" --out /out/int
  docker run --rm --user "$(id -u):$(id -g)" -v "$(dirname "$LOINC_ZIP"):/in:ro" -v "$out_dir:/out" \
    --entrypoint /usr/local/bin/ferroterm-build "$image" --loinc "/in/$(basename "$LOINC_ZIP")" --out /out/loinc
  grep -qE "\"storeLayout\" *: *\"?$want" "$out_dir/int/manifest.json" "$out_dir/loinc/manifest.json" \
    || fail "the rebuilt index is not layout $want"
  rsync -a --delete "$out_dir/" "$SSH_USER@$APP_HOST:$APP_DIR/ferroterm-index.new/"
  app <<EOF
set -euo pipefail
cd "$APP_DIR"
rm -rf ferroterm-index.prev
mv ferroterm-index ferroterm-index.prev
mv ferroterm-index.new ferroterm-index
docker compose up -d ferroterm
EOF
  ok "rebuilt both indexes with $image and swapped them in (previous: $APP_DIR/ferroterm-index.prev); update $INDEX_RECORD in the same change"
}

# ── step 5: after the v5.5.0 deploy ───────────────────────────────────────────
wait_healthy() {
  app <<EOF
deadline=\$(( \$(date +%s) + 600 ))
while :; do
  bad=""
  for c in ferroehr-sandbox ferroehr-sandbox-ferropix ferroehr-sandbox-ferroterm ferroehr-sandbox-viewer; do
    s="\$(docker inspect --format '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "\$c" 2>/dev/null | sed '/^\$/d')" || true
    s="\${s:-absent}"
    [ "\$s" = "running healthy" ] || bad="\$bad \$c=\$s"
  done
  if [ -z "\$bad" ]; then echo "all healthy"; exit 0; fi
  if [ "\$(date +%s)" -ge "\$deadline" ]; then echo "not healthy after 10 minutes:\$bad"; exit 1; fi
  sleep 10
done
EOF
}

# The pseudonyms: the box registers every manifest subject with FerroPIX
# (an idempotent conditional update) and answers each pseudonym, which
# FerroPIX keeps; then the demo data is seeded under them unless it already is.
seed_identities() {
  {
    printf 'cd %q\n' "$APP_DIR"
    printf 'SSH_ORIGINAL_COMMAND=identities ./deploy.sh <<%s\n' "'SUBJECTS'"
    cat "$WORK/subjects"
    echo "SUBJECTS"
  } | remote "$APP_HOST" > "$WORK/identities" || fail "the identities verb failed; its stderr is above"
  grep -q '^provider ihe$' "$WORK/identities" || fail "the box answers $(head -n 1 "$WORK/identities"): the CDR posture does not resolve through FerroPIX"
  local count
  count="$(grep -c '^subject ' "$WORK/identities" || true)"
  [[ "$count" -eq "$(wc -l < "$WORK/subjects" | tr -d ' ')" ]] \
    || fail "FerroPIX answered $count pseudonyms for $(wc -l < "$WORK/subjects" | tr -d ' ') subjects"
  sed 's/^/  identities: /' "$WORK/identities"
  ok "FerroPIX holds all $count demo patients, each with one pseudonym in the clinical domain"

  local base="$PUBLIC_URL/ferroehr/rest/openehr/v1" ns marker code
  ns="$(sed -n 's/^pseudonym_namespace //p' "$WORK/identities")"
  marker="$(awk -v m="$(jq -r '.marker.subject' "$MANIFEST")" '$2 == m { print $3 }' "$WORK/identities")"
  code="$(curl -sS -o /dev/null -w "$HTTP_STATUS" -u "$DEMO_AUTH" "$base/ehr?subject_id=$marker&subject_namespace=$ns")"
  if [[ "$code" == 200 ]]; then
    ok "the demo data is seeded under the pseudonyms (marker EHR present)"
  else
    # No finished seed under the pseudonyms: the CDR holds nothing, the demo
    # data seeded before FerroPIX, or a seed that stopped part way. Wipe and
    # restart as the nightly reset does, then seed. A definition an
    # unfinished seed stored answers 409 on the next run and counts as stored,
    # so seeding over it moves the archetype-library split (#4278).
    printf 'SSH_ORIGINAL_COMMAND=wipe %q/deploy.sh 2>&1\n' "$APP_DIR" | remote "$APP_HOST" > "$WORK/wipe" \
      || fail "the wipe verb failed: $(tail -n 5 "$WORK/wipe")"
    echo "  app: wipe dropped $(grep -c '^DROP SCHEMA' "$WORK/wipe" || true) schemas"
    printf 'SSH_ORIGINAL_COMMAND=restart %q/deploy.sh 2>&1\n' "$APP_DIR" | remote "$APP_HOST" | tail -n 2 | indent_app
    ok "wiped the sandbox schemas and restarted the CDR"
    local deadline=$(($(date +%s) + 600))
    until [[ "$(curl -sS -m 30 -o /dev/null -w "$HTTP_STATUS" "$PUBLIC_URL/health/readiness" || true)" == 200 ]]; do
      [[ "$(date +%s)" -lt "$deadline" ]] || fail "the CDR was not ready 10 minutes after the restart"
      sleep 15
    done
    SANDBOX_BASE="$PUBLIC_URL" SANDBOX_IDENTITIES="$WORK/identities" bash "$ROOT/scripts/sandbox/reseed.sh" \
      | grep -E '^==>|::error' | sed 's/^/  seed: /' || true
    code="$(curl -sS -o /dev/null -w "$HTTP_STATUS" -u "$DEMO_AUTH" "$base/ehr?subject_id=$marker&subject_namespace=$ns")"
    [[ "$code" == 200 ]] || fail "the seed did not finish (marker lookup answered $code); the seed lines above say where it stopped"
    ok "seeded the demo data under the pseudonyms"
  fi

}

step5() {
  step 5 "after the v$RELEASE_FLOOR deploy: recreate, seed FerroPIX, verify end to end"
  local version
  version="$(running_cdr_version)"
  if ! release_knows_ferropix "$version"; then
    skip "the box runs FerroEHR ${version:-nothing}; run step 5 again once the v$RELEASE_FLOOR release has deployed"
    return
  fi
  ok "the box runs FerroEHR $version"

  local out
  if [[ "$CHECK" -eq 0 ]]; then
    app > "$WORK/out" 2>&1 <<EOF || fail "the recreate failed: $(tail -n 20 "$WORK/out")"
set -euo pipefail
cd "$APP_DIR"
if ! grep -q '^  ferropix:' docker-compose.yml; then
  # The release's deploy did not install the FerroPIX posture (it refuses one
  # whose .env names are unset); install it now from the image.
  SSH_ORIGINAL_COMMAND=deploy ./deploy.sh
  echo "installed the posture with the deploy verb"
else
  docker compose pull -q
  # A recreate, never a restart (a restart keeps the old environment). The
  # namespace members go first so the issuer can be replaced.
  docker compose rm --stop --force ferroehr ferropix
  docker compose up -d --remove-orphans
  echo "recreated the stack"
fi
EOF
    tail -n 8 "$WORK/out" | indent_app
  fi
  out="$(wait_healthy)" || fail "$out"
  ok "$out"
  app > "$WORK/out" <<'EOF'
for c in ferroehr-sandbox ferroehr-sandbox-ferropix ferroehr-sandbox-ferroterm ferroehr-sandbox-viewer ferroehr-sandbox-issuer; do
  docker inspect --format "$c {{.Config.Image}} {{index .Config.Labels \"org.opencontainers.image.version\"}}" "$c"
done
docker stats --no-stream --format '{{.Name}} {{.MemUsage}}' | grep ferroehr-sandbox
EOF
  out="$(cat "$WORK/out")"
  printf '%s\n' "$out" | indent_app
  local ferroterm_version
  ferroterm_version="$(awk '$1 == "ferroehr-sandbox-ferroterm" && NF == 3 && $2 ~ /^ghcr/ { print $3; exit }' <<<"$out")"
  [[ "$ferroterm_version" == "$FERROTERM_PIN_VERSION" ]] \
    || fail "FerroTERM runs ${ferroterm_version:-nothing}, the posture pins $FERROTERM_PIN_VERSION; a FERROTERM_IMAGE line in .env overrides it (step 3 retires it)"
  ok "FerroTERM runs the pinned $FERROTERM_PIN_VERSION"
  local ferropix_version
  ferropix_version="$(awk '$1 == "ferroehr-sandbox-ferropix" && NF == 3 && $2 ~ /^ghcr/ { print $3; exit }' <<<"$out")"
  [[ "$ferropix_version" == "$FERROPIX_PIN_VERSION" ]] \
    || fail "FerroPIX runs ${ferropix_version:-nothing}, the posture pins $FERROPIX_PIN_VERSION; deploy the release whose posture pins it, and retire a FERROPIX_IMAGE line in .env (step 3)"
  ok "FerroPIX runs the pinned $FERROPIX_PIN_VERSION"

  # The reseed's archetype-library pin holds only while the CDR resolves LOINC
  # (scripts/sandbox/seed/manifest.json, #4278): ask before seeding.
  local probe probe_code
  probe="$(jq -r '.adl2.archetype_library.loinc.probe' "$MANIFEST")"
  probe_code="$(curl -sS -o "$WORK/loinc.out" -w "$HTTP_STATUS" -u "$DEMO_AUTH" \
    "$PUBLIC_URL/ferroehr/rest/extensions/v1/terminology/http%3A%2F%2Floinc.org/term/$probe")"
  [[ "$probe_code" == 200 ]] \
    || fail "the CDR resolves no LOINC (LOINC $probe answered $probe_code): FerroTERM opens no LOINC index, so the seed's archetype-library pin cannot hold; check FERROTERM_INDEXES in .env (step 3) and the index (step 4)"
  ok "FerroTERM serves LOINC through the CDR: $probe is $(jq -r --arg c "$probe" '.terms[$c].text // "?"' "$WORK/loinc.out")"

  jq -r '.ehrs[].subject, .marker.subject' "$MANIFEST" > "$WORK/subjects"
  if [[ "$CHECK" -eq 1 ]]; then
    skip "--check registers no patient with FerroPIX; the cohort below proves the resolution"
  else
    seed_identities
  fi

  local ext="$PUBLIC_URL/ferroehr/rest/extensions/v1" code
  # The CDR resolves the hospital identifiers through FerroPIX: a cohort named
  # by the source domain's identifiers lands on the EHRs whose EHR_STATUS
  # carries the pseudonyms FerroPIX answered.
  jq -nc --arg ns "$SOURCE_DOMAIN" --rawfile s "$WORK/subjects" '{
      q: "SELECT e/ehr_id/value, e/ehr_status/subject/external_ref/id/value FROM EHR e",
      subjects: [$s | split("\n")[] | select(startswith("sandbox-patient")) | {id: ., namespace: $ns}]
    }' > "$WORK/cohort.json"
  code="$(curl -sS -o "$WORK/cohort.out" -w "$HTTP_STATUS" -u "$DEMO_AUTH" \
    -H 'Content-Type: application/json' -X POST "$ext/query/cohort" --data-binary @"$WORK/cohort.json")"
  [[ "$code" == 200 ]] || fail "POST /query/cohort answered $code: $(head -c 300 "$WORK/cohort.out")"
  local rows matched pattern=' [0-9a-f-]{36}$'
  rows="$(jq '.rows | length' "$WORK/cohort.out")"
  # Each EHR must carry a pseudonym: on a run, the one FerroPIX just answered;
  # under --check, which asks FerroPIX nothing, a UUID.
  [[ -s "$WORK/identities" ]] || jq -r '.rows[][1] | "subject - " + .' "$WORK/cohort.out" > "$WORK/identities"
  matched="$(jq -r '.rows[][1]' "$WORK/cohort.out" | while read -r p; do
    grep -E "$pattern" "$WORK/identities" | grep -c " $p\$" > /dev/null && echo "$p"
  done | wc -l | tr -d ' ')"
  [[ "$rows" -ge 5 && "$rows" == "$matched" ]] \
    || fail "the cohort answered $rows rows, $matched of them on FerroPIX's pseudonyms"
  ok "the CDR resolved the hospital identifiers through FerroPIX: $rows EHRs, each on its pseudonym in the clinical domain"

  code="$(curl -sS -o "$WORK/term.out" -w "$HTTP_STATUS" -u "$DEMO_AUTH" "$ext/terminology/urn:iso:std:iso:3166/term/NL")"
  { [[ "$code" == 200 ]] && grep -q Netherlands "$WORK/term.out"; } \
    || fail "FerroTERM did not answer through the CDR ($code): $(head -c 300 "$WORK/term.out")"
  ok "FerroTERM answers through the CDR: urn:iso:std:iso:3166 NL is $(jq -r '.terms.NL.text' "$WORK/term.out")"

  local status
  status="$(curl -sS -m 30 "$PUBLIC_URL/ferroehr/rest/status" | jq -r '.server_version // empty')"
  [[ -n "$status" ]] || fail "$PUBLIC_URL/ferroehr/rest/status served no server_version"
  local landing
  landing="$(curl -fsSL -m 30 "$PUBLIC_URL/" || true)"
  grep -q FerroEHR <<<"$landing" || fail "$PUBLIC_URL/ does not serve the viewer"
  ok "$PUBLIC_URL serves FerroEHR $status and the viewer"
}

# ── step 6: the DB box onto the pinned image ──────────────────────────────────
# The values a quoted box script reads, as the assignments it starts with.
db_vars() {
  printf 'WANT=%q DIR=%q CONTAINER=%q CHECK=%q\n' "$DB_IMAGE" "$DB_DIR" "$DB_CONTAINER" "$CHECK"
}

# What the DB box runs, and under a run whether the pin keeps its major.
db_image_state() {
  { db_vars; cat <<'EOF'; } | db
set -euo pipefail
cd "$DIR"
psql() { docker exec "$CONTAINER" psql -U postgres -Atc "$1"; }
running="$(docker inspect --format '{{.Config.Image}}' "$CONTAINER")"
num="$(psql 'SHOW server_version_num')"
echo "running=$running"
echo "server=$(psql 'SHOW server_version')"
echo "server_major=$((num / 10000))"
echo "volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$CONTAINER" | sed 's/ $//')"
echo "env_image=$(sed -n 's/^FERROEHR_POSTGRES_IMAGE=//p' .env | tail -n 1)"
echo "compose_image=$(sed -nE 's/^[[:space:]]*image:[[:space:]]*//p' docker-compose.yml | tr '\n' ' ')"
# A runtime may record the pin without its tag, as the repository and digest.
if [ "$running" = "$WANT" ] || [ "$running" = "${WANT%%:*}@${WANT#*@}" ]; then
  echo "verdict=current"
  exit 0
fi
# A run pulls the pin to read its major; --check reads it only when present.
[ "$CHECK" -eq 1 ] || docker pull -q "$WANT" > /dev/null
major="$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$WANT" 2>/dev/null \
  | sed -n 's/^PG_MAJOR=//p')" || true
echo "image_major=${major:-unknown}"
if [ -z "$major" ]; then
  echo "verdict=$([ "$CHECK" -eq 1 ] && echo behind || echo no-major)"
elif [ "$major" != "$((num / 10000))" ]; then
  echo "verdict=major"
else
  echo "verdict=behind"
fi
EOF
}

# Points the DB box's compose project at the pin and recreates PostgreSQL.
db_image_swap() {
  { db_vars; cat <<'EOF'; } | db
set -euo pipefail
cd "$DIR"
[ "$(grep -cE '^[[:space:]]*image:' docker-compose.yml)" -eq 1 ] || { echo "verdict=layout"; exit 0; }
cp -p .env .env.prev
cp -p docker-compose.yml docker-compose.yml.prev
# The compose file names the image through FERROEHR_POSTGRES_IMAGE, as
# cloud-init-postgres.yaml writes it, with the pin as its default.
awk -v want="$WANT" '/^[[:space:]]*image:/ {
  match($0, /^[[:space:]]*/)
  print substr($0, 1, RLENGTH) "image: ${FERROEHR_POSTGRES_IMAGE:-" want "}"
  next
} { print }' docker-compose.yml.prev > docker-compose.yml.next
cat docker-compose.yml.next > docker-compose.yml
rm -f docker-compose.yml.next
umask 077
{ grep -v '^FERROEHR_POSTGRES_IMAGE=' .env.prev || true; echo "FERROEHR_POSTGRES_IMAGE=$WANT"; } > .env.next
mv .env.next .env
chmod 600 .env
configured="$(docker compose config --images 2>/dev/null | sed -n 1p)" || true
if [ "$configured" != "$WANT" ]; then
  cp -p .env.prev .env
  cp -p docker-compose.yml.prev docker-compose.yml
  echo "configured=${configured:-nothing}"
  echo "verdict=config"
  exit 0
fi
log="$(mktemp)"
# A recreate, never a restart: the container takes the new image and keeps
# its named volume.
if ! { docker compose pull -q && docker compose up -d; } > "$log" 2>&1; then
  sed 's/^/compose: /' "$log"
  echo "verdict=compose"
  exit 0
fi
rm -f "$log"
deadline=$(($(date +%s) + 300))
until [ "$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$CONTAINER" 2>/dev/null)" = healthy ]; do
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "verdict=unhealthy"
    exit 0
  fi
  sleep 5
done
psql() { docker exec "$CONTAINER" psql -U postgres -Atc "$1"; }
echo "running=$(docker inspect --format '{{.Config.Image}}' "$CONTAINER")"
echo "server=$(psql 'SHOW server_version')"
echo "server_major=$(($(psql 'SHOW server_version_num') / 10000))"
echo "volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$CONTAINER" | sed 's/ $//')"
echo "databases=$(psql "SELECT string_agg(datname, ',' ORDER BY datname) FROM pg_database WHERE NOT datistemplate")"
echo "verdict=swapped"
EOF
}

# 0 when the image reference $1 a container records names the pin.
runs_pin() { [[ "$1" == "$DB_IMAGE" || "$1" == "${DB_IMAGE%%:*}@${DB_IMAGE#*@}" ]]; }

# The value of `$1=` in the box output $2.
field() { sed -n "s/^$1=//p" <<<"$2" | tail -n 1; }

step6() {
  step 6 "DB box: PostgreSQL onto the pinned ferroehr-postgres image"
  [[ "$DB_IMAGE" =~ ^ghcr\.io/ferrohealth/ferroehr-postgres:[0-9]+\.[0-9]+\.[0-9]+@sha256:[0-9a-f]{64}$ ]] \
    || fail "$CLOUD_INIT_DB pins no release image by digest (found '$DB_IMAGE')"
  local out before verdict
  db_image_state > "$WORK/out" || fail "the DB box refused the step: $(cat "$WORK/out")"
  before="$(cat "$WORK/out")"
  grep -v '^verdict=' <<<"$before" | sed 's/^/  db: /'
  verdict="$(field verdict "$before")"
  case "$verdict" in
    current) ok "the DB box runs the pin $DB_IMAGE" ;;
    major)
      fail "the pin carries PostgreSQL $(field image_major "$before") and the box runs $(field server_major "$before"): a major upgrade needs pg_upgrade, so nothing was changed"
      ;;
    no-major) fail "the pin declares no PG_MAJOR, so the step cannot prove the major stays; nothing was changed" ;;
    behind)
      [[ "$CHECK" -eq 0 ]] \
        || fail "the DB box runs $(field running "$before"), not the pin $DB_IMAGE (the pin carries PostgreSQL $(field image_major "$before"), the box runs $(field server_major "$before")); run step 6 without --check"
      [[ -n "$(field volume "$before")" ]] \
        || fail "the container keeps its data in no volume, which a recreate would lose; nothing was changed"
      ok "the pin keeps PostgreSQL $(field server_major "$before")"
      echo "  note: the sandbox is unavailable for about a minute while PostgreSQL restarts on the new image"
      db_image_swap > "$WORK/out" || fail "the swap failed on the DB box: $(cat "$WORK/out")"
      out="$(cat "$WORK/out")"
      grep -v '^verdict=' <<<"$out" | sed 's/^/  db: /'
      local back="cd $DB_DIR && cp -p .env.prev .env && cp -p docker-compose.yml.prev docker-compose.yml && docker compose up -d"
      case "$(field verdict "$out")" in
        swapped) ;;
        layout) fail "$DB_DIR/docker-compose.yml names more or fewer than one image; nothing was changed" ;;
        config) fail "compose resolves '$(field configured "$out")', not the pin; the previous files are restored" ;;
        compose) fail "docker compose could not pull or recreate (its output is above); to go back: $back" ;;
        unhealthy) fail "PostgreSQL was not healthy 5 minutes after the recreate; to go back: $back" ;;
        *) fail "the swap gave no verdict; to go back: $back" ;;
      esac
      runs_pin "$(field running "$out")" || fail "the container runs $(field running "$out") after the recreate"
      [[ "$(field volume "$out")" == "$(field volume "$before")" ]] \
        || fail "the data volume changed from $(field volume "$before") to $(field volume "$out")"
      [[ "$(field server_major "$out")" == "$(field server_major "$before")" ]] \
        || fail "PostgreSQL moved from major $(field server_major "$before") to $(field server_major "$out")"
      ok "recreated on $DB_IMAGE: PostgreSQL $(field server "$out"), volume $(field volume "$out") kept, databases $(field databases "$out") (previous: $DB_DIR/.env.prev, $DB_DIR/docker-compose.yml.prev)"
      ;;
    *) fail "the DB box gave no verdict" ;;
  esac

  # The proof from the side that matters: the CDR is ready on the database,
  # and FerroPIX's role logs in over the private network.
  local deadline=$(($(date +%s) + 300)) code started
  started="$(date +%s)"
  until code="$(curl -sS -m 10 -o /dev/null -w "$HTTP_STATUS" "$PUBLIC_URL/health/readiness" 2> /dev/null)" && [[ "$code" == 200 ]]; do
    [[ "$(date +%s)" -lt "$deadline" ]] || fail "$PUBLIC_URL/health/readiness answered ${code:-nothing} for 5 minutes"
    sleep 5
  done
  ok "the CDR is ready: $PUBLIC_URL/health/readiness answered 200 after $(($(date +%s) - started))s"
  local pw bind
  db > "$WORK/out" <<EOF
sed -n 's/^FERROPIX_DB_PASSWORD=//p' "$DB_DIR/.env" | tail -n 1
sed -n 's/^PG_BIND_IP=//p' "$DB_DIR/.env" | tail -n 1
EOF
  pw="$(sed -n 1p "$WORK/out")"
  bind="$(sed -n 2p "$WORK/out")"
  [[ -n "$pw" ]] || fail "no FERROPIX_DB_PASSWORD on the DB box; run step 2 first"
  out="$(ferropix_login "$pw" "$bind")"
  judge_login "$out" "$bind"
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  self_test
  exit
fi
command -v jq > /dev/null || fail "jq is required on this machine"
[[ -f "$MANIFEST" ]] || fail "run this from a checkout: $MANIFEST is missing"
release="$(release_verdict "$(served_version || true)" "$(checkout_version)")" || fail "$release"
ok "$release"
echo "sandbox-ferropix-rollout: app box $SSH_USER@$APP_HOST, DB box $SSH_USER@$DB_HOST, $([[ "$CHECK" -eq 1 ]] && echo 'check only' || echo 'changing what is missing'), steps ${STEPS[*]}"
want_step 1 && step1
want_step 2 && step2
want_step 3 && step3
want_step 4 && step4
want_step 5 && step5
want_step 6 && step6
printf '\nsandbox-ferropix-rollout: done\n'
