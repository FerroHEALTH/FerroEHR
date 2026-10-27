#!/bin/sh
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Register the sandbox's demo patients with FerroPIX and answer their
# pseudonyms (#4242). The compose file's one-shot `ferropix-seed` runs this
# inside the development issuer's network namespace, where FerroPIX and the
# issuer answer on loopback; the deploy script's `identities` verb starts it
# and feeds it the manifest subjects on stdin, one per line.
#
# Each subject is fed as the Patient Identity Source `sandbox-source` with a
# conditional update (PIXm ITI-104, idempotent: a re-run updates the same
# patient), then asked as the consumer `ferroehr` for its identifier in the
# clinical domain (ITI-83), which mints the pseudonym on the first ask and
# answers the same one ever after. The patients are synthetic: a family name
# and the subject as their identifier, nothing else.
#
# Output, the file reseed.sh reads as SANDBOX_IDENTITIES:
#   provider ihe
#   pseudonym_namespace <namespace>
#   subject <subject> <pseudonym>
# Anything the source refuses stops the run with a non-zero exit.
set -eu

issuer="http://localhost:8180/sandbox"
pix="http://localhost:8090/fhir"
source_system="urn:oid:2.999.1.1"
clinical="urn:oid:2.999.7.1"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A client credentials token from the development issuer, which signs any
# request: the client id and the scope are what FerroPIX checks.
token() {
  curl -fsS -m 30 -d grant_type=client_credentials -d "client_id=$1" \
    -d client_secret=sandbox -d "scope=$2" "$issuer/token" \
    | sed -n 's/.*"access_token" *: *"\([^"]*\)".*/\1/p'
}

src="$(token sandbox-source system/Patient.cud)"
consumer="$(token ferroehr system/Patient.rs)"
if [ -z "$src" ] || [ -z "$consumer" ]; then
  echo "ferropix-seed: the issuer answered no access token" >&2
  exit 1
fi

echo "provider ihe"
echo "pseudonym_namespace $clinical"
count=0
while IFS= read -r subject; do
  [ -n "$subject" ] || continue
  case "$subject" in
    *[!A-Za-z0-9._-]*)
      echo "ferropix-seed: refusing subject '$subject'" >&2
      exit 1
      ;;
  esac
  code="$(curl -sS -m 30 -o "$work/feed" -w '%{http_code}' -X PUT \
    -H "Authorization: Bearer $src" -H 'Content-Type: application/fhir+json' \
    "$pix/Patient?identifier=$source_system%7C$subject" \
    --data-binary "{\"resourceType\":\"Patient\",\"identifier\":[{\"system\":\"$source_system\",\"value\":\"$subject\"}],\"name\":[{\"family\":\"Sandbox\",\"given\":[\"$subject\"]}]}")"
  case "$code" in
    200 | 201) ;;
    *)
      echo "ferropix-seed: the feed of $subject answered $code" >&2
      cat "$work/feed" >&2
      exit 1
      ;;
  esac
  code="$(curl -sS -m 30 -o "$work/pix" -w '%{http_code}' \
    -H "Authorization: Bearer $consumer" -H 'Accept: application/fhir+json' \
    "$pix/Patient/\$ihe-pix?sourceIdentifier=$source_system%7C$subject&targetSystem=$clinical")"
  if [ "$code" != 200 ]; then
    echo "ferropix-seed: the cross-reference of $subject answered $code" >&2
    cat "$work/pix" >&2
    exit 1
  fi
  # The pseudonym is the value of the one targetIdentifier whose system is the
  # clinical domain (ITI-83 §2:3.83.4.2.2), whatever the key order.
  pseudonym="$(tr -d ' \n' < "$work/pix" \
    | grep -oE '\{[^{}]*"system":"'"$clinical"'"[^{}]*\}' \
    | grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' | sort -u)"
  if [ "$(printf '%s\n' "$pseudonym" | grep -c .)" != 1 ]; then
    echo "ferropix-seed: the cross-reference of $subject answered no single pseudonym" >&2
    exit 1
  fi
  echo "subject $subject $pseudonym"
  count=$((count + 1))
done
if [ "$count" -eq 0 ]; then
  echo "ferropix-seed: no subject on stdin" >&2
  exit 1
fi
echo "ferropix-seed: $count patients registered and cross-referenced" >&2
