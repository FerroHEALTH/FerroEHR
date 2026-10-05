#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The PGP signing probe family (#2163's second mode).
#
# #2163 is explicit that the PGP path must be exercised "with a real key,
# including the passphrase-file route, since that is the documented production
# shape". So this generates a real OpenPGP key, mounts it the way the
# configuration reference instructs — `signing.key_path` for the armored secret
# key, `signing.key_passphrase_file` for its passphrase — and drives the same
# assertions the digest family does, including tamper detection.
#
# The key is generated into a THROWAWAY keyring under the run's temp directory:
# a probe must never touch the operator's own GnuPG home.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

PGP_PASS="probe-passphrase-not-a-secret"

# Generate an armored secret key, its PUBLIC half and its passphrase file into a
# named directory, echoing that directory. Empty output means gpg was
# unavailable or refused.
#
# The public half is exported because key ROTATION needs it: `retired_key_paths`
# takes public keys, so a retired key can verify history and can never sign
# again.
pgp_material() {
  local name="${1:-pgp}"
  local dir="$PROBE_TMP/$name"
  mkdir -p "$dir" "$dir/gnupg"
  chmod 700 "$dir/gnupg"
  local fpr
  GNUPGHOME="$dir/gnupg" gpg --batch --pinentry-mode loopback \
    --passphrase "$PGP_PASS" --quick-generate-key \
    "FerroEHR Probe $name <probe-$name@example.test>" default default never >/dev/null 2>&1 || return 1
  fpr="$(GNUPGHOME="$dir/gnupg" gpg --batch --list-secret-keys --with-colons 2>/dev/null \
    | awk -F: '/^fpr:/ {print $10; exit}')"
  [[ -n "$fpr" ]] || return 1
  GNUPGHOME="$dir/gnupg" gpg --batch --pinentry-mode loopback \
    --passphrase "$PGP_PASS" --armor --export-secret-keys "$fpr" > "$dir/signing.asc" 2>/dev/null || return 1
  GNUPGHOME="$dir/gnupg" gpg --batch --armor --export "$fpr" > "$dir/public.asc" 2>/dev/null || return 1
  [[ -s "$dir/signing.asc" ]] && [[ -s "$dir/public.asc" ]] || return 1
  printf '%s' "$PGP_PASS" > "$dir/passphrase"
  # The container runs as the distroless nonroot uid; these are throwaway probe
  # credentials, so world-readable is the simplest way to guarantee the mount is
  # readable regardless of host uid mapping.
  chmod 644 "$dir/signing.asc" "$dir/public.asc" "$dir/passphrase"
  printf '%s' "$dir"
}

# An overlay mounting that material and switching the server to pgp mode. It is
# GENERATED rather than committed: the key does not exist until the run makes
# one, and a committed signing key in a repository is its own defect.
pgp_overlay() {
  local dir="$1" out="$PROBE_TMP/pgp-overlay.yml"
  cat > "$out" <<YAML
services:
  ferroehr:
    volumes:
      - $dir/signing.asc:/etc/ferroehr/signing.asc:ro
      - $dir/passphrase:/run/secrets/pgp-pass:ro
    environment:
      FERROEHR__SIGNING__ENABLED: "true"
      FERROEHR__SIGNING__MODE: pgp
      FERROEHR__SIGNING__KEY_PATH: /etc/ferroehr/signing.asc
      FERROEHR__SIGNING__KEY_PASSPHRASE_FILE: /run/secrets/pgp-pass
YAML
  printf '%s' "$out"
}

probes_signing_pgp() {
  bold "VERSION signing (PGP mode, key + passphrase from mounted files)"

  if ! command -v gpg >/dev/null 2>&1; then
    uncovered "signing in PGP mode (#2163)" "gpg is not available on this host"
    return 0
  fi
  local dir overlay
  if ! dir="$(pgp_material)" || [[ -z "$dir" ]]; then
    uncovered "signing in PGP mode (#2163)" "this host's gpg would not generate a probe key"
    return 0
  fi
  overlay="$(pgp_overlay "$dir")"

  # `pgp` mode fails CLOSED at boot when the key is missing or unusable, so the
  # server coming up at all is the first assertion — and the documented
  # production shape (a mounted key, a passphrase from its *_file sibling) is
  # what is being mounted here.
  probe "P-PGP-BOOT" "working" "server" "#2163" \
    "the server boots in pgp mode with the key and passphrase from mounted files"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 120; then
    probe_fail "a serving CDR in pgp mode" "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)" \
      "pgp mode fails closed at boot, so this is the key or the passphrase route"
    probe_done
    return 0
  fi
  probe_done

  probe "P-PGP-SIGN" "working" "server" "#2163" \
    "a version committed under pgp carries a signature and verifies on read"
  local ehr version
  ehr="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
    | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$ehr" ]]; then
    probe_fail "a committed EHR" "no id returned"
    probe_done
    return 0
  fi
  version="$(curl -s -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")"
  assert_contains "$version" '"signature"' "a pgp-signed version must carry its detached signature"
  # An OpenPGP detached signature is armored; digest mode's is not. Asserting
  # the armor is what distinguishes "pgp is really in use" from "signing is on".
  assert_contains "$version" "BEGIN PGP SIGNATURE" \
    "without the armor this is not an OpenPGP signature, whatever the mode says"
  assert_eq "200" "$(http_code -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")" \
    "an untampered pgp-signed version verifies on read"
  probe_done

  # Detection, in the mode where it is hardest to fake.
  probe "P-PGP-TAMPER" "broken" "server" "#2163" \
    "a tampered pgp-signed version is REFUSED on read"
  local matched after
  probe_psql "UPDATE clinical.version
                 SET body = (jsonb_set((body)::jsonb, '{name,value}', '\"tampered\"'))::text
               WHERE ehr_id = '$ehr'::uuid AND kind = 'EHR_STATUS';" >/dev/null
  matched="$(probe_psql "SELECT count(*) FROM clinical.version
                          WHERE ehr_id = '$ehr'::uuid
                            AND (body)::jsonb #>> '{name,value}' = 'tampered';")"
  if [[ "${matched:-0}" = "0" ]]; then
    probe_fail "a tampered stored row" "the UPDATE matched nothing" \
      "detection was never tested — the probe could not reach the stored content"
  else
    after="$(http_code -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")"
    case "$after" in
      5*) : ;;
      *)  probe_fail "a 5xx integrity refusal" "$after" \
            "verify_on_read is strict by default; a corrupt pgp-signed record must not be served" ;;
    esac
  fi
  probe_done

  # Put the stack back on the shipped posture for anything that follows.
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true
}

# The rotation overlay: sign with `signer`, and keep `retired` public keys for
# verification only.
pgp_rotation_overlay() {
  local signer="$1" retired="$2" out="$PROBE_TMP/pgp-rotate-overlay.yml"
  cat > "$out" <<YAML
services:
  ferroehr:
    volumes:
      - $signer/signing.asc:/etc/ferroehr/signing.asc:ro
      - $signer/passphrase:/run/secrets/pgp-pass:ro
      - $retired/public.asc:/etc/ferroehr/retired.pub.asc:ro
    environment:
      FERROEHR__SIGNING__ENABLED: "true"
      FERROEHR__SIGNING__MODE: pgp
      FERROEHR__SIGNING__KEY_PATH: /etc/ferroehr/signing.asc
      FERROEHR__SIGNING__KEY_PASSPHRASE_FILE: /run/secrets/pgp-pass
      FERROEHR__SIGNING__RETIRED_KEY_PATHS: /etc/ferroehr/retired.pub.asc
YAML
  printf '%s' "$out"
}

# Key rotation: #2122's acceptance criteria, driven live.
#
# #2122 recorded that rotating a PGP key made every previously-signed version
# fail verification — a 5xx while reading intact historical clinical data — and
# was closed by adding a verification keyring (`retired_key_paths`). These
# probes check the SHIPPED FIX rather than the original defect, and they check
# both halves of it: history keeps verifying, AND verification did not simply
# become permissive.
probes_signing_rotation() {
  bold "VERSION signing — key rotation (the #2122 keyring)"

  if ! command -v gpg >/dev/null 2>&1; then
    uncovered "PGP key rotation (#2122)" "gpg is not available on this host"
    return 0
  fi
  local key_a key_b overlay ehr_a
  if ! key_a="$(pgp_material rotate-a)" || [[ -z "$key_a" ]] \
     || ! key_b="$(pgp_material rotate-b)" || [[ -z "$key_b" ]]; then
    uncovered "PGP key rotation (#2122)" "this host's gpg would not generate the two probe keys"
    return 0
  fi

  # Sign a version with key A.
  dc -f docker-compose.yml -f "$(pgp_overlay "$key_a")" up -d ferroehr >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 120; then
    probe "P-ROT-SETUP" "working" "server" "#2122" "a version is signed under key A"
    probe_fail "a serving CDR under key A" "readiness never returned"
    probe_done
    return 0
  fi
  ehr_a="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
    | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"

  # Rotate: sign with B, keep A's PUBLIC key for verification only.
  overlay="$(pgp_rotation_overlay "$key_b" "$key_a")"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr >/dev/null 2>&1

  probe "P-ROT-BOOT" "working" "server" "#2122" \
    "the server boots after a rotation with the retired key kept for verification"
  if ! wait_http "$CDR/health/readiness" 120; then
    probe_fail "a serving CDR after rotation" "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)" \
      "retired_key_paths takes PUBLIC keys; a rejected one fails closed at boot"
    probe_done
    return 0
  fi
  probe_done

  # #2122's third criterion, verbatim: sign with A, rotate to B, read the
  # key-A version back under strict verification without an integrity failure.
  probe "P-ROT-HISTORY" "working" "server" "#2122" \
    "a version signed by the RETIRED key still verifies after rotation"
  if [[ -z "$ehr_a" ]]; then
    probe_fail "a version signed under key A" "no EHR was committed before the rotation"
  else
    assert_eq "200" "$(http_code -u "$BASIC" "$API/ehr/$ehr_a/versioned_ehr_status/version")" \
      "reading intact history after a rotation must not be an integrity failure"
  fi
  probe_done

  # The other half of #2122's second criterion: keeping history verifiable must
  # not make verification permissive. A keyring that accepts anything would pass
  # the probe above and be worthless.
  probe "P-ROT-STILL-STRICT" "broken" "server" "#2122" \
    "a tampered version still fails after rotation — the keyring is not permissive"
  local matched after
  probe_psql "UPDATE clinical.version
                 SET body = (jsonb_set((body)::jsonb, '{name,value}', '\"tampered\"'))::text
               WHERE ehr_id = '$ehr_a'::uuid AND kind = 'EHR_STATUS';" >/dev/null
  matched="$(probe_psql "SELECT count(*) FROM clinical.version
                          WHERE ehr_id = '$ehr_a'::uuid
                            AND (body)::jsonb #>> '{name,value}' = 'tampered';")"
  if [[ "${matched:-0}" = "0" ]]; then
    probe_fail "a tampered stored row" "the UPDATE matched nothing" \
      "permissiveness was never tested — the probe could not reach the stored content"
  else
    after="$(http_code -u "$BASIC" "$API/ehr/$ehr_a/versioned_ehr_status/version")"
    case "$after" in
      5*) : ;;
      *)  probe_fail "a 5xx integrity refusal" "$after" \
            "a signature matching NO key in the ring must still fail; the point is verifiable history, not permissive reads" ;;
    esac
  fi
  probe_done

  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true
}
