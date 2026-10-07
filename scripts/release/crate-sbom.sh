#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A CycloneDX SBOM per published `openehr-*` crate, bound to the `.crate`
# archive crates.io serves.
#
# CRA Annex I Part II(1) asks for an SBOM "covering at the very least the
# top-level dependencies of the products" (docs/law/eu/cra/text.html), and each
# crate is a product with its own support period. The release lane runs this
# per crate, attests the SBOM against the archive, and attaches both to the
# release; the crates leg later proves the archive crates.io serves is the one
# that was attested.
#
# The archive is the attestation subject because it is what a consumer
# downloads (`cargo` verifies it against the index checksum). A version crates.io
# already holds is fetched from there and checked against that checksum. A new
# version is packaged here from the release commit; `cargo package` writes
# deterministic archive headers (cargo src/ops/cargo_package, the
# DETERMINISTIC_TIMESTAMP comment), so the crates leg's `cargo publish` uploads
# the same bytes, and `verify-published` refuses the release record otherwise.
#
# Usage:
#   crate-sbom.sh archive <crate> <version>   # writes <crate>-<version>.crate
#   crate-sbom.sh sbom <crate> <version>      # writes <crate>-<version>.cdx.json
#   crate-sbom.sh verify-published <version>  # every crate's crates.io archive
#                                             # carries this repository's SBOM
#                                             # attestation
#
# Requires: cargo, cargo-cyclonedx, curl, jq, sha256sum; `verify-published`
# additionally requires gh with GH_TOKEN.
set -euo pipefail
cd "$(dirname "$0")/../.."

# shellcheck source=scripts/gh/retry-net.sh
. scripts/gh/retry-net.sh

die() {
  echo "::error::crate-sbom: $*" >&2
  exit 1
}

# The crates the release publishes: the one list publish-crates.sh keeps.
crate_list() {
  bash scripts/release/publish-crates.sh list
}

known_crate() {
  crate_list | grep -Fxq "$1" || die "'$1' is not one of the published crates ($(crate_list | tr '\n' ' '))"
}

# The crates.io checksum of <crate> <version>, or nothing when the version is
# not published. A failed request is fatal: an unknown answer must not be read
# as "not published".
published_checksum() {
  local body
  body="$(retry_read "https://crates.io/api/v1/crates/$1/versions" \
    -H 'User-Agent: ferroehr-release-sbom')" || die "crates.io did not answer for $1"
  jq -r --arg v "$2" '.versions[]? | select(.num == $v) | .checksum' <<<"$body"
}

do_archive() {
  local crate="$1" version="$2" out="$1-$2.crate" checksum
  known_crate "$crate"
  checksum="$(published_checksum "$crate" "$version")"
  if [[ -n "$checksum" ]]; then
    retry_read "https://static.crates.io/crates/${crate}/${out}" -o "$out" \
      || die "could not download ${out} from crates.io"
    echo "${checksum}  ${out}" | sha256sum -c - >/dev/null \
      || die "${out} from static.crates.io does not match the index checksum ${checksum}"
    echo "${out}: the archive crates.io already serves (sha256 ${checksum})"
    return
  fi
  # Every listed crate is packaged together, so a crate whose sibling is also
  # unpublished at this version resolves it from the same run.
  local -a packages=()
  local c
  while IFS= read -r c; do packages+=(-p "$c"); done < <(crate_list)
  cargo package "${packages[@]}" --no-verify --locked
  [[ -f "target/package/${out}" ]] || die "cargo package wrote no target/package/${out}"
  cp "target/package/${out}" "$out"
  echo "${out}: packaged from this commit, not yet on crates.io (sha256 $(sha256sum "$out" | cut -d' ' -f1))"
}

do_sbom() {
  local crate="$1" version="$2" out="$1-$2.cdx.json"
  known_crate "$crate"
  local dir="crates/${crate}"
  [[ -f "${dir}/Cargo.toml" ]] || die "no ${dir}/Cargo.toml"
  find "$dir" -maxdepth 1 -name '*.cdx.json' -delete
  # `--spec-version 1.5` is the highest cargo-cyclonedx emits (it accepts 1.3,
  # 1.4 or 1.5 and defaults to 1.3). Dev-dependencies are omitted by the tool,
  # which matches what `cargo package` strips.
  cargo cyclonedx --manifest-path "${dir}/Cargo.toml" \
    --format json --describe crate --spec-version 1.5
  local -a written=()
  local f
  while IFS= read -r f; do written+=("$f"); done < <(find "$dir" -maxdepth 1 -name '*.cdx.json')
  [[ "${#written[@]}" -eq 1 ]] || die "expected one SBOM in ${dir}, found ${#written[@]}"
  mv "${written[0]}" "$out"
  # Refuse a document that is not CycloneDX or describes another crate.
  jq -e --arg c "$crate" --arg v "$version" '
    .bomFormat == "CycloneDX"
    and .metadata.component.name == $c
    and .metadata.component.version == $v' "$out" >/dev/null \
    || die "${out} is not a CycloneDX SBOM of ${crate} ${version}"
  echo "${out}: CycloneDX $(jq -r '.specVersion' "$out"), $(jq '.components | length' "$out") components"
}

do_verify_published() {
  local version="$1" crate out bad=""
  local work
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local and gone at EXIT time
  trap "rm -rf '$work'" EXIT
  while IFS= read -r crate; do
    out="${work}/${crate}-${version}.crate"
    if ! retry_read "https://static.crates.io/crates/${crate}/${crate}-${version}.crate" -o "$out"; then
      echo "${crate}: ${version} is not downloadable from crates.io"
      bad="${bad} ${crate}"
      continue
    fi
    if gh attestation verify "$out" --repo "${GITHUB_REPOSITORY:?}" \
      --predicate-type https://cyclonedx.org/bom \
      --signer-workflow "${GITHUB_REPOSITORY}/.github/workflows/release.yml" >/dev/null; then
      echo "${crate}: the crates.io archive carries this repository's SBOM attestation"
    else
      echo "${crate}: NO SBOM attestation matches the archive crates.io serves"
      bad="${bad} ${crate}"
    fi
  done < <(crate_list)
  [[ -z "$bad" ]] || die "the released SBOMs do not describe the archives crates.io serves for:${bad}"
}

case "${1:-}" in
archive)
  [[ "$#" -eq 3 ]] || die "usage: $0 archive <crate> <version>"
  do_archive "$2" "$3"
  ;;
sbom)
  [[ "$#" -eq 3 ]] || die "usage: $0 sbom <crate> <version>"
  do_sbom "$2" "$3"
  ;;
verify-published)
  [[ "$#" -eq 2 ]] || die "usage: $0 verify-published <version>"
  do_verify_published "$2"
  ;;
*)
  echo "usage: $0 archive <crate> <version> | sbom <crate> <version> | verify-published <version>" >&2
  exit 2
  ;;
esac
