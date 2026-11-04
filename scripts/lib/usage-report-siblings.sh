#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The sibling-server half of every family repository's
# scripts/checks/usage-report-off.sh: the checks that a launcher which starts
# another family server (FerroEHR, FerroPIX, FerroTERM, FerroFED, FerroBRIDGE)
# hands that server its own switch, `<PRODUCT>__USAGE_REPORT__ENABLED`.
#
# This file is the family's reference copy and FerroEHR owns it. FerroPIX,
# FerroFED, FerroTERM and FerroBRIDGE carry it verbatim at
# scripts/lib/usage-report-siblings.sh, so a fix lands here first and the
# others copy the file whole; it holds nothing particular to one repository.
# What differs per repository comes from the caller's arguments: its own key,
# the paths its guard exempts, its candidate launchers and its file listing.
# No specification governs this: our own design.
#
# Source it from the guard as:
#
#   # shellcheck source=scripts/lib/usage-report-siblings.sh
#   . "$(dirname "$0")/../lib/usage-report-siblings.sh"
#
# and call `sibling_findings OWN_KEY EXEMPT_RE CANDIDATES LISTING...`.

# The family products, and the image reference of a family server.
readonly SIBLING_FAMILY='ferroehr|ferropix|ferroterm|ferrofed|ferrobridge'
readonly SIBLING_FAMILY_IMAGE="ghcr\\.io/ferrohealth/($SIBLING_FAMILY)[:@]"

# Prints the file's content without comment lines, one logical line per
# backslash-continued command.
sibling_logical_lines() {
  local file="$1"
  awk '/^[[:space:]]*#/ { next }
       { l = $0; if (sub(/\\$/, "", l)) { buf = buf l " "; next } print buf l; buf = "" }
       END { if (buf != "") print buf }' "$file"
}

# Prints `file: service (KEY)` for each service of the Compose file $2 that
# runs a family server image without that server's switch in its own
# `environment`: KEY set to false, or to an interpolation of the caller's key
# $1, which every Compose launcher exports as false. A service that merges an
# `x-` anchor (`<<: *name`) runs the anchor's image; one whose `entrypoint`
# names another binary of the image, or whose `command` names a subcommand
# other than `serve`, starts no server.
sibling_compose_findings() {
  local own="$1" file="$2"
  awk -v own="$own" -v img="$SIBLING_FAMILY_IMAGE" -v file="$file" -v sq="'" '
    function close_block() {
      if (kind == "anchor" && product != "") anchor_image[name] = product
      if (kind == "service") { svc[++n] = name; svc_image[name] = product; svc_merge[name] = merge; svc_entry[name] = entry; svc_cmd[name] = cmd }
      kind = ""; name = ""; product = ""; merge = ""; entry = ""; cmd = ""
    }
    /^[[:space:]]*#/ { next }
    /^[^[:space:]]/ {
      close_block(); in_services = ($0 ~ /^services:/)
      if (match($0, /&[A-Za-z0-9_-]+/)) { kind = "anchor"; name = substr($0, RSTART + 1, RLENGTH - 1) }
      next
    }
    in_services && /^  [A-Za-z0-9_.-]+:/ {
      close_block(); kind = "service"; name = $0; sub(/^  /, "", name); sub(/:.*/, "", name); next
    }
    kind == "" { next }
    /^[[:space:]]+image:/ && match($0, img) { product = toupper(substr($0, RSTART + 20, RLENGTH - 21)) }
    /^[[:space:]]+entrypoint:/ { entry = $0 }
    /^[[:space:]]+command:/ { cmd = $0; sub(/^[[:space:]]+command:[[:space:]]*\[?[[:space:]]*["\047]?/, "", cmd); sub(/[^-A-Za-z0-9_].*/, "", cmd) }
    match($0, /<<:[[:space:]]*\*[A-Za-z0-9_-]+/) { merge = substr($0, RSTART, RLENGTH); sub(/.*\*/, "", merge) }
    match($0, /[A-Z]+__USAGE_REPORT__ENABLED[[:space:]]*[:=]/) {
      key = substr($0, RSTART, RLENGTH); sub(/[[:space:]]*[:=]$/, "", key)
      value = substr($0, RSTART + RLENGTH); gsub(sq, "", value); gsub(/^[[:space:]"]+|[[:space:]"]+$/, "", value)
      if (kind == "service" && (value == "false" || index(value, "${" own) > 0)) ok[name, key] = 1
    }
    END {
      close_block()
      for (i = 1; i <= n; i++) {
        s = svc[i]; p = svc_image[s]
        if (p == "" && svc_merge[s] != "") p = anchor_image[svc_merge[s]]
        # An entrypoint naming another binary of the image (an index build) serves nothing.
        if (p != "" && svc_entry[s] != "" && svc_entry[s] !~ ("/" tolower(p) "([^-a-z]|$)")) continue
        # So does a `command` naming a subcommand other than serve (`cdm init`).
        if (p != "" && svc_cmd[s] != "" && svc_cmd[s] !~ /^-/ && svc_cmd[s] != "serve") continue
        if (p != "" && !((s, p "__USAGE_REPORT__ENABLED") in ok))
          printf "%s: %s (%s__USAGE_REPORT__ENABLED)\n", file, s, p
      }
    }
  ' "$file"
}

# Prints `file (KEY)` for each `docker run` in $2 that starts a family server
# other than the caller's ($1 is its key) without handing the container that
# server's switch on its own command line. A one-shot job starts no server.
sibling_run_findings() {
  local own="$1" file="$2" line product key
  while IFS= read -r line; do
    product="$(grep -oE "$SIBLING_FAMILY_IMAGE" <<<"$line" | sed -nE '1{s|.*/||; s|[:@]$||; p;}' | tr '[:lower:]' '[:upper:]')"
    key="${product}__USAGE_REPORT__ENABLED"
    [[ -n "$product" && "$key" != "$own" ]] || continue
    grep -qE '[[:space:]](healthcheck|--version|--help|config[[:space:]]+check)([[:space:]]|$)' <<<"$line" && continue
    grep -qE "(-e|--env)[[:space:]=]+[\"']?${key}=false" <<<"$line" || printf '%s (%s)\n' "$file" "$key"
  done < <(sibling_logical_lines "$file" | grep -E "docker[[:space:]]+run[[:space:]].*$SIBLING_FAMILY_IMAGE" || true)
}

# Prints `crate (KEY)` for each crate whose Rust source names a family server
# image other than the caller's (`"ghcr.io/ferrohealth/<product>"`) while no
# Rust source of that crate sets `"<PRODUCT>__USAGE_REPORT__ENABLED", "false"`
# on the container. $1 = the caller's key; the rest = the file-listing
# command.
sibling_rust_findings() {
  local own="$1" files f crate product key
  shift
  files="$("$@" | grep -E '\.rs$' || true)"
  [[ -n "$files" ]] || return 0
  # shellcheck disable=SC2086 # one word per tracked path, none with spaces
  while IFS= read -r f; do
    crate="$(cut -d/ -f1-2 <<<"$f")"
    while IFS= read -r product; do
      key="$(tr '[:lower:]' '[:upper:]' <<<"$product")__USAGE_REPORT__ENABLED"
      [[ "$key" != "$own" ]] || continue
      # shellcheck disable=SC2046 # one word per tracked path, none with spaces
      grep -qE "\"${key}\",[[:space:]]*\"false\"" $(grep -E "^${crate}/" <<<"$files") ||
        printf '%s (%s)\n' "$crate" "$key"
    done < <(grep -oE "\"ghcr\\.io/ferrohealth/($SIBLING_FAMILY)\"" "$f" | sed -E 's|.*/||; s|"$||' | sort -u)
  done < <(grep -lE "\"ghcr\\.io/ferrohealth/($SIBLING_FAMILY)\"" $files 2>/dev/null || true)
}

# Prints the Compose files among the listed ones: YAML with a top-level
# `services:`, outside .github/, scripts/checks/ and the paths the extended
# regular expression $1 matches. The rest = the file-listing command.
sibling_compose_files() {
  local exempt="$1" f
  shift
  while IFS= read -r f; do
    case "$f" in
      .github/* | scripts/checks/*) continue ;;
      *.yml | *.yaml)
        [[ -n "$exempt" ]] && grep -qE "$exempt" <<<"$f" && continue
        grep -qE '^services:' "$f" 2>/dev/null && printf '%s\n' "$f"
        ;;
      *) ;;
    esac
  done < <("$@")
}

# Prints every sibling finding: $1 = the caller's own key, $2 = the extended
# regular expression of the Compose paths its guard exempts (empty for none),
# $3 = its candidate launchers, one per line; the rest = the file-listing
# command.
sibling_findings() {
  local own="$1" exempt="$2" candidates="$3" f
  shift 3
  while IFS= read -r f; do sibling_compose_findings "$own" "$f"; done < <(sibling_compose_files "$exempt" "$@")
  while IFS= read -r f; do [[ -n "$f" ]] && sibling_run_findings "$own" "$f"; done <<<"$candidates"
  sibling_rust_findings "$own" "$@"
}
