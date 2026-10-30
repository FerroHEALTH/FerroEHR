#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Browser-driven login against the BUILT viewer image (issue #2871).
#
# The asset-chain smoke beside this one proves the pushed image SERVES every
# FerroUI asset its own page references. Nothing proved a browser can log in
# through them: the host `scripts/ui-e2e.sh` battery drives a browser, but at a
# viewer built on the runner, never at the shipped artifact. This
# closes that gap by driving real Chrome over the W3C WebDriver protocol
# (curl + jq — this repository authors no JavaScript) and asserting BOTH
# halves of a working login:
#
#   1. the shipped scripts attach: FerroUI's loader marks the root element
#      once it has imported the page's modules; a viewer whose scripts never
#      load still logs in through the server-rendered form (the #2164
#      class), so a login alone passes on a broken asset chain;
#   2. the browser posts the credentials through the sign-in card's form and
#      lands on the authenticated Dashboard. The Basic form is a plain POST
#      to the page itself, `/login`, which answers accepted credentials with
#      a redirect and refused ones with the page drawn again (#3980). So the
#      script checks the form posts to /login before the click, and after it
#      reads the performance log for that Document POST and its 3xx answer:
#      a Dashboard reached by any other path (an automatic sign-in, a
#      session already present) shows no such redirect and fails.
#
# Required (no defaults — a gate that passes without its artifact is not a
# gate):
#   FERROEHR_VIEWER_IMAGE   the viewer image under test
#   FERROEHR_IMAGE            the CDR the viewer authenticates against
#   FERROEHR_POSTGRES_IMAGE   the CDR's database
# The two CDR images are FIXTURES; the viewer image is the artifact under test.
# CHROMEDRIVER, else CHROMEWEBDRIVER (the GitHub runner images' variable),
# else `chromedriver` on PATH selects the driver binary.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for var in FERROEHR_VIEWER_IMAGE FERROEHR_IMAGE FERROEHR_POSTGRES_IMAGE; do
  if [[ -z "${!var:-}" ]]; then
    echo "FATAL: $var is unset — this gate must run against explicit images" >&2
    exit 1
  fi
done

if [[ -n "${CHROMEDRIVER:-}" ]]; then
  DRIVER_BIN="$CHROMEDRIVER"
elif [[ -n "${CHROMEWEBDRIVER:-}" ]]; then
  DRIVER_BIN="$CHROMEWEBDRIVER/chromedriver"
else
  DRIVER_BIN="chromedriver"
fi
command -v "$DRIVER_BIN" >/dev/null 2>&1 || {
  echo "FATAL: no chromedriver at '$DRIVER_BIN' (set CHROMEDRIVER)" >&2
  exit 1
}

# Own compose project + the viewer's profile on EVERY call, so the teardown
# below can see the viewer container and can never reach another stack
# (docs.docker.com/compose/how-tos/project-name, /profiles).
export COMPOSE_PROJECT_NAME=ferroehr-ui-login-smoke
export COMPOSE_PROFILES=viewer
# A test instance sends no usage report (the quickstart reads this switch).
export FERROEHR__USAGE_REPORT__ENABLED=false
# The standalone quickstart file only: an explicit -f suppresses the automatic
# override merge, so this drives what a downloader runs
# (docs.docker.com/compose/how-tos/multiple-compose-files).
COMPOSE=(docker compose -f "$ROOT_DIR/docker-compose.yml")
VIEWER_PORT="${FERROEHR_VIEWER_PORT:-3000}"
VIEWER_URL="http://127.0.0.1:${VIEWER_PORT}"
DRIVER_URL="http://127.0.0.1:9515"
DRIVER_PID=""
SESSION=""

cleanup() {
  if [[ -n "$SESSION" ]]; then
    curl -sS -X DELETE "$DRIVER_URL/session/$SESSION" >/dev/null 2>&1 || true
  fi
  [[ -n "$DRIVER_PID" ]] && kill "$DRIVER_PID" 2>/dev/null || true
  echo "::group::viewer logs"
  "${COMPOSE[@]}" logs ferroehr-viewer || true
  echo "::endgroup::"
  "${COMPOSE[@]}" down -v --remove-orphans || true
}
trap cleanup EXIT

echo "── compose up (postgres + CDR + the viewer image under test)"
echo "   viewer: $FERROEHR_VIEWER_IMAGE"
"${COMPOSE[@]}" up -d --wait ferroehr-postgres ferroehr ferroehr-viewer

echo "── chromedriver: $DRIVER_BIN"
"$DRIVER_BIN" --port=9515 --allowed-ips=127.0.0.1 >/tmp/chromedriver.log 2>&1 &
DRIVER_PID=$!
for _ in $(seq 1 30); do
  if curl -sf "$DRIVER_URL/status" | jq -e '.value.ready' >/dev/null 2>&1; then break; fi
  sleep 1
done
curl -sf "$DRIVER_URL/status" | jq -e '.value.ready' >/dev/null || {
  echo "FATAL: chromedriver never became ready" >&2
  cat /tmp/chromedriver.log >&2
  exit 1
}

# The performance log shows the Document POST of the sign-in form; the
# browser log carries any module error.
SESSION=$(curl -sS -X POST "$DRIVER_URL/session" \
  -H 'Content-Type: application/json' \
  -d '{"capabilities":{"alwaysMatch":{"browserName":"chrome",
        "goog:loggingPrefs":{"browser":"ALL","performance":"ALL"},
        "goog:chromeOptions":{
          "args":["--headless=new","--disable-gpu","--window-size=1440,900"],
          "perfLoggingPrefs":{"enableNetwork":true}}}}}' \
  | jq -r '.value.sessionId // empty')
[[ -n "$SESSION" ]] || {
  echo "FATAL: chromedriver refused a session" >&2
  cat /tmp/chromedriver.log >&2
  exit 1
}
S="$DRIVER_URL/session/$SESSION"

# A found element is a single-key object keyed by a spec-fixed UUID; a missing
# one is an object carrying `error`, which must NOT read as an element id
# (w3c.github.io/webdriver — Web Element, Handling Errors).
find_element() {
  curl -sS -X POST "$S/element" -H 'Content-Type: application/json' \
    -d "{\"using\":\"css selector\",\"value\":\"$1\"}" \
    | jq -r '.value | select(type == "object") | select(has("error") | not)
             | to_entries[0].value'
}
send_keys() {
  local out
  out=$(curl -sS -X POST "$S/element/$1/value" -H 'Content-Type: application/json' \
    -d "{\"text\":\"$2\"}")
  jq -e '.value == null' >/dev/null <<<"$out" || {
    echo "FATAL: typing into the login form was refused: $(jq -c '.value' <<<"$out")" >&2
    exit 1
  }
}
current_url() { curl -sS "$S/url" | jq -r '.value'; }
dump_browser_log() {
  echo "── browser log"
  curl -sS -X POST "$S/se/log" -H 'Content-Type: application/json' \
    -d '{"type":"browser"}' | jq -r '.value[]? | "\(.level) \(.message)"' || true
}

echo "── driving the login form at $VIEWER_URL/login"
curl -sS -X POST "$S/url" -H 'Content-Type: application/json' \
  -d "{\"url\":\"$VIEWER_URL/login\"}" >/dev/null

USER_EL=""
for _ in $(seq 1 30); do
  USER_EL=$(find_element 'input[name=username]')
  [[ -n "$USER_EL" ]] && break
  sleep 1
done
[[ -n "$USER_EL" ]] || {
  echo "FATAL: the viewer never rendered a username input" >&2
  dump_browser_log
  exit 1
}
# The page is server-rendered and its form works with no script by design.
# Wait (bounded) for the mark FerroUI's loader writes on the root element
# once it has imported the page's modules (`data-fui-script`); scripts that
# never load fail here, with their own message.
SCRIPTED=""
for _ in $(seq 1 30); do
  SCRIPTED=$(find_element 'html[data-fui-script]')
  [[ -n "$SCRIPTED" ]] && break
  sleep 1
done
[[ -n "$SCRIPTED" ]] || {
  echo "::error::FerroUI's scripts never attached to the viewer page within 30 s (the #2164 class)"
  dump_browser_log
  exit 1
}
PASS_EL=$(find_element 'input[name=password]')
FORM_EL=$(find_element 'form:has(input[name=password])')
SUBMIT_EL=$(find_element 'form:has(input[name=password]) button[type=submit]')
[[ -n "$PASS_EL" && -n "$FORM_EL" && -n "$SUBMIT_EL" ]] || {
  echo "FATAL: the sign-in card has no form holding the password input and a submit button" >&2
  exit 1
}
# The IDL `method` and `action` properties are the form's effective ones: the
# keyword lowercased and the URL resolved against the page (HTML Living
# Standard, form submission attributes).
FORM_METHOD=$(curl -sS "$S/element/$FORM_EL/property/method" | jq -r '.value')
FORM_ACTION=$(curl -sS "$S/element/$FORM_EL/property/action" | jq -r '.value')
echo "   sign-in form: method=$FORM_METHOD action=$FORM_ACTION"
[[ "$FORM_METHOD" == post && "$FORM_ACTION" == "$VIEWER_URL/login" ]] || {
  echo "::error::the sign-in card's form does not POST to $VIEWER_URL/login (method=$FORM_METHOD action=$FORM_ACTION)"
  dump_browser_log
  exit 1
}

# The quickstart Basic user the compose `configs:` block carries inline.
send_keys "$USER_EL" ferroehr
send_keys "$PASS_EL" ferroehr
# A real click, never form.submit(): the reported defect was a hydrated submit
# listener that consumed the click and dispatched nothing, which only a click
# through the button can observe.
CLICK=$(curl -sS -X POST "$S/element/$SUBMIT_EL/click" \
  -H 'Content-Type: application/json' -d '{}')
jq -e '.value == null' >/dev/null <<<"$CLICK" || {
  echo "FATAL: the submit click was refused: $(jq -c '.value' <<<"$CLICK")" >&2
  dump_browser_log
  exit 1
}

URL=""
for _ in $(seq 1 20); do
  URL="$(current_url)"
  case "$URL" in */login*) sleep 1 ;; *) break ;; esac
done
TITLE=$(curl -sS "$S/title" | jq -r '.value')
echo "   url=$URL"
echo "   title=$TITLE"

# Read the performance log once (reading drains it). A redirect keeps the
# request id and arrives as a second requestWillBeSent carrying the previous
# hop's `redirectResponse` (Chrome DevTools Protocol, Network domain), so one
# line per POST to /login reads `<type> <has form body> <redirect status>`.
LOGIN_POSTS=$(curl -sS -X POST "$S/se/log" -H 'Content-Type: application/json' \
  -d '{"type":"performance"}' \
  | jq -r '.value[]?.message' \
  | jq -rs --arg url "$VIEWER_URL/login" '
      [.[] | .message | select(.method == "Network.requestWillBeSent") | .params] as $sent
      | $sent[]
      | select(.request.method == "POST" and .request.url == $url)
      | . as $post
      | ([$sent[] | select(.requestId == $post.requestId and .redirectResponse.url == $url)
                  | .redirectResponse.status][0] // "none") as $status
      | "\($post.type) \($post.request.hasPostData // false) \($status)"' || true)
echo "   login POSTs (type, form body, redirect status): ${LOGIN_POSTS:-<none>}"

case "$URL" in
  */login*)
    echo "::error::the viewer image cannot be logged into from a browser: the submit click left the page on /login (issue #2871)"
    dump_browser_log
    exit 1
    ;;
esac
case "$TITLE" in
  Dashboard*) ;;
  *)
    echo "::error::the login landed on '$URL' with title '$TITLE', not the Dashboard"
    dump_browser_log
    exit 1
    ;;
esac
grep -qE '^Document true 3[0-9]{2}$' <<<"$LOGIN_POSTS" || {
  echo "::error::no Document POST of the sign-in form to /login was answered with a redirect (${LOGIN_POSTS:-no login POST observed}): the login reached the Dashboard by another path"
  dump_browser_log
  exit 1
}

dump_browser_log
echo "── the viewer image logs in through a real browser: scripts attached, sign-in form POST redirected, Dashboard reached"
