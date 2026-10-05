#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the EU legal acts the compliance pages cite into docs/law/eu/, one
# directory per act, in the format its publisher serves.
#
# WHY THE TEXTS ARE IN THE TREE. The compliance pages make claims against
# named articles of named acts. A claim checked against a page fetched at
# review time is checked against whatever that page says on the day, and
# neither the reviewer nor a later reader can tell whether the text moved
# underneath the citation. A consolidation vendored at a named date is a fixed
# referent: the citation resolves to bytes this repository carries, and the
# SHA256SUMS beside them proves they are the bytes this script fetched.
#
# FORMAT AND ENDPOINT. The English act is fetched as XHTML from the
# Publications Office content-negotiation URI,
# publications.europa.eu/resource/celex/<celex> with
# `Accept: application/xhtml+xml`, and vendored verbatim as text.html. Nothing
# is converted, reformatted or extracted: an extraction is an edit, and an
# edited act is no longer the publisher's text.
#
# It is deliberately NOT the eur-lex.europa.eu web page, which serves the same
# document wrapped in site chrome that includes a bot-detection script tag
# carrying a per-request agent id. Two fetches of the identical, unchanged act
# then differ, so a re-run could never reproduce the vendored bytes and the
# recorded digest would stop meaning anything. The Cellar URI answers the same
# document byte for byte on every request (measured across repeated fetches
# when the pins were set), which is the whole property this corpus is built on.
#
# The EDPB publishes its guidelines as PDF only, so the PDF is vendored as
# published (guidelines.pdf) with that fact recorded in its provenance.
#
# WHICH CELEX. An act whose amendments have been folded in is pinned at its
# CONSOLIDATED CELEX and the consolidation date; an act with no amendments is
# pinned at the authentic OJ text, because for those two the substantive text
# is the same and only the OJ version has legal effect (a consolidation carries
# EUR-Lex's own "no legal effect" disclaimer). Each act's PROVENANCE.md records
# which of the two it is and why.
#
# LICENSING. The Commission's reuse policy (Decision 2011/833/EU) covers the
# legal documents; the consolidated texts are additionally the EU's own
# editorial content under CC BY 4.0, and the EDPB states its own reuse terms.
# All three are quoted verbatim in LICENSES/, declared per directory in
# REUSE.toml, and named in each PROVENANCE.md.
#
# Idempotent: each act directory is wiped and re-fetched. Re-run after changing
# a pin below; the SHA-256 lines move with the bytes, and docs/law/README.md is
# the index that must move with them.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO_ROOT/docs/law/eu"

# The publisher sees a named client rather than a default curl string, so a
# fetch from this repository is attributable at the far end.
UA='ferroehr-vendor/1.0 (+https://github.com/FerroHEALTH/FerroEHR)'
FETCHED="$(date -u +%Y-%m-%d)"

# directory | CELEX | ELI of the base act | consolidation date | byte floor | title
#
# The byte floor is roughly two thirds of the size measured when the pin was
# set. It is not a checksum — it is the difference between an act and the
# login, consent or error page a publisher answers with when something goes
# wrong, which is otherwise a plausible-looking file nobody notices.
ACTS=(
  "gdpr|02016R0679-20160504|http://data.europa.eu/eli/reg/2016/679/oj|2016-05-04|300000|Regulation (EU) 2016/679 — General Data Protection Regulation"
  "ehds|32025R0327|http://data.europa.eu/eli/reg/2025/327/oj||500000|Regulation (EU) 2025/327 — European Health Data Space"
  "nis2|32022L2555|http://data.europa.eu/eli/dir/2022/2555/oj||400000|Directive (EU) 2022/2555 — measures for a high common level of cybersecurity across the Union (NIS2)"
  "cra|32024R2847|http://data.europa.eu/eli/reg/2024/2847/oj||400000|Regulation (EU) 2024/2847 — horizontal cybersecurity requirements for products with digital elements (Cyber Resilience Act)"
  "mdr|02017R0745-20260719|http://data.europa.eu/eli/reg/2017/745/oj|2026-07-19|1000000|Regulation (EU) 2017/745 — medical devices (Medical Device Regulation)"
)

# Why each act is here, in one sentence, written into its PROVENANCE.md.
act_reason() {
  case "$1" in
  gdpr) printf '%s' "The processing law every deployment answers to: the lawfulness, minimisation, security and rights obligations the pseudonymisation boundary, the access log and the records of processing are built against." ;;
  ehds) printf '%s' "The health-data regulation whose Chapter III puts requirements on an EHR system itself, including the Annex II logging elements the audit trail is measured against." ;;
  nis2) printf '%s' "The cybersecurity directive that binds the essential and important entities a deployment of this software typically is, and the incident-reporting regime around it." ;;
  cra) printf '%s' "The horizontal cybersecurity regulation for products with digital elements, which EHDS Chapter III cross-references for the essential requirements an EHR system inherits." ;;
  mdr) printf '%s' "The medical-device regulation behind the Article 27 interoperability question the EHDS readiness page leaves open for a deployment that claims interoperability with a device." ;;
  *) printf '%s' "" ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# One line per vendored file, in the format `sha256sum -c` / `shasum -a 256 -c`
# reads, with the path relative to the act directory so the check runs there.
# The provenance record and the sums file are excluded: they describe the
# vendored bytes rather than being them.
write_sha256sums() {
  local dir="$1" name
  : >"$dir/SHA256SUMS"
  while IFS= read -r name; do
    printf '%s  %s\n' "$(sha256_of "$dir/$name")" "$name" >>"$dir/SHA256SUMS"
  done < <(cd "$dir" && find . -type f ! -name SHA256SUMS ! -name PROVENANCE.md | sed 's|^\./||' | sort)
}

# $1 url, $2 destination, $3 byte floor, $4 a word the real document must
# contain (empty to skip), $5 the media type to negotiate (empty for none).
# Prints the byte count; refuses everything else.
fetch() {
  local url="$1" out="$2" floor="$3" needle="$4" accept="$5" code bytes
  local -a headers=()
  if [[ -n "$accept" ]]; then
    headers+=(-H "Accept: $accept" -H "Accept-Language: eng")
  fi
  code="$(curl -sS -A "$UA" -L --proto '=https' --proto-redir '=https' "${headers[@]+"${headers[@]}"}" -o "$out" -w '%{http_code}' "$url")"
  if [[ "$code" != 200 ]]; then
    echo "ERROR: $url answered HTTP $code" >&2
    exit 1
  fi
  bytes="$(wc -c <"$out" | tr -d ' ')"
  if [[ "$bytes" -lt "$floor" ]]; then
    echo "ERROR: $url returned $bytes bytes, under the $floor-byte floor — that is a login, consent or error page, not an act" >&2
    exit 1
  fi
  if [[ -n "$needle" ]] && ! grep -qi -- "$needle" "$out"; then
    echo "ERROR: $url returned $bytes bytes that never say '$needle' — that is not the act text" >&2
    exit 1
  fi
  printf '%s' "$bytes"
}

# The CELEX URI answers 303 with the document's own Cellar address, and that
# Location header is spelled `http://`. Following it would be a plaintext hop,
# so the redirect is resolved here and re-issued over https instead of handing
# curl a `--proto-redir` that permits the downgrade. The resolved address names
# the exact manifestation (its Cellar id and revision), which is worth keeping:
# the pin stays at the CELEX, and the record says which manifestation that CELEX
# resolved to on the fetch date.
resolve_celex() {
  local uri="$1" location
  location="$(curl -sS -A "$UA" --proto '=https' -D - -o /dev/null \
    -H 'Accept: application/xhtml+xml' -H 'Accept-Language: eng' "$uri" \
    | awk 'tolower($1) == "location:" { sub(/\r$/, "", $2); print $2 }' | tail -n 1)"
  if [[ -z "$location" ]]; then
    echo "ERROR: $uri did not redirect to a document" >&2
    exit 1
  fi
  printf '%s' "https://${location#*://}"
}

mkdir -p "$DEST"

for entry in "${ACTS[@]}"; do
  IFS='|' read -r dir celex eli consolidated floor title <<<"$entry"
  celex_uri="https://publications.europa.eu/resource/celex/$celex"
  url="$(resolve_celex "$celex_uri")"
  page="https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:$celex"
  out="$DEST/$dir"
  echo "==> eu/$dir (CELEX:$celex)"
  rm -rf "$out"
  mkdir -p "$out"

  bytes="$(fetch "$url" "$out/text.html" "$floor" "Article" "application/xhtml+xml")"
  digest="$(sha256_of "$out/text.html")"

  if [[ -n "$consolidated" ]]; then
    version_note="Consolidated text at **$consolidated**, the version in force
on the fetch date. EUR-Lex prints its own disclaimer on a consolidation: it
\"is meant purely as a documentation tool and has no legal effect\". A citation
that has to be authoritative is therefore checked against the OJ act and its
amendments. This file is what the act says today, which is what a compliance
claim is read against."
    licence_line="Commission reuse policy (Decision 2011/833/EU) for the act, and
  CC BY 4.0 for the consolidated editorial layer, which the EU owns. Both are
  quoted verbatim in \`LICENSES/LicenseRef-EUR-Lex-Reuse.txt\` and
  \`LICENSES/CC-BY-4.0.txt\`, and declared for this directory in \`REUSE.toml\`."
  else
    version_note="Not a consolidated text: this is the act as published in the
Official Journal. EUR-Lex lists no consolidation for it that resolves (checked
when the pin was set), and where an initial consolidation does exist it folds
in no amendment, so the authentic OJ text is pinned instead of a documentation-tool
version that carries a no-legal-effect disclaimer for the same words."
    licence_line="Commission reuse policy (Decision 2011/833/EU), quoted verbatim
  in \`LICENSES/LicenseRef-EUR-Lex-Reuse.txt\` and declared for this directory
  in \`REUSE.toml\`."
  fi

  cat >"$out/PROVENANCE.md" <<EOF
# $title

$(act_reason "$dir")

| | |
|---|---|
| CELEX | \`$celex\` |
| ELI (base act) | $eli |
| Consolidation vendored | ${consolidated:-none — the OJ text} |
| Fetched from | $celex_uri (\`Accept: application/xhtml+xml\`) |
| Resolved to | $url |
| Human-readable at | $page |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-eu.sh\` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`text.html\` | $bytes | \`$digest\` |

The English XHTML the Publications Office serves for this CELEX, byte for byte.
Nothing was converted or extracted. The same document is what the EUR-Lex page
above renders; that page is not the fetch source because it wraps the act in
site chrome carrying a per-request identifier, which would make the digest
above unreproducible.

## Version

$version_note

## Licence

- $licence_line
- Copyright: © European Union, 1998-$(date -u +%Y). Reuse is authorised
  provided the source is acknowledged; this record and the index at
  \`docs/law/README.md\` are that acknowledgement.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-eu.sh\` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.
EOF

  write_sha256sums "$out"
  echo "    text.html: $bytes bytes"
done

# ── The EDPB guidelines: PDF only ──────────────────────────────────────────
# The EDPB publishes its guidelines as PDF and nothing else. There is no HTML,
# XML or plain-text edition to prefer, so the PDF is vendored as published
# rather than converted — a conversion would be this repository's rendering of
# somebody else's document, and the clause numbering a citation resolves
# against would then be ours.
EDPB_DIR="$DEST/edpb-guidelines-01-2025-pseudonymisation"
EDPB_URL="https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf"
EDPB_PAGE="https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en"
echo "==> eu/edpb-guidelines-01-2025-pseudonymisation"
rm -rf "$EDPB_DIR"
mkdir -p "$EDPB_DIR"
edpb_bytes="$(fetch "$EDPB_URL" "$EDPB_DIR/guidelines.pdf" 300000 "" "")"
if [[ "$(head -c 5 "$EDPB_DIR/guidelines.pdf")" != "%PDF-" ]]; then
  echo "ERROR: $EDPB_URL did not answer with a PDF" >&2
  exit 1
fi
edpb_digest="$(sha256_of "$EDPB_DIR/guidelines.pdf")"
cat >"$EDPB_DIR/PROVENANCE.md" <<EOF
# EDPB Guidelines 01/2025 on pseudonymisation

The supervisory authorities' own reading of what pseudonymisation is under
GDPR Art. 4(5) and what it does to the risk analysis under Art. 32 — the
document the pseudonymisation boundary in this software is designed against.
Guidelines are not law: they bind nobody, and this record says so rather than
letting a vendored PDF read like an act.

| | |
|---|---|
| Adopted | 16 January 2025 (version 1.0, for public consultation) |
| Document page | $EDPB_PAGE |
| Source | $EDPB_URL |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-eu.sh\` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`guidelines.pdf\` | $edpb_bytes | \`$edpb_digest\` |

**No text format is published.** The EDPB serves this document as PDF and
offers no HTML, XML or plain-text edition of it, so the PDF is vendored as
published. It is not converted: a converted copy would be this repository's
rendering, and a paragraph number cited against it would resolve to our
pagination rather than the EDPB's.

## Licence

Reuse of EDPB material is authorised for commercial and non-commercial
purposes on the conditions its copyright page states — acknowledge the source,
do not distort the meaning, and the EDPB carries no liability for the reuse.
The page is quoted verbatim in
\`LICENSES/LicenseRef-EDPB-Reuse.txt\` and declared for this directory in
\`REUSE.toml\`. Source acknowledged: European Data Protection Board,
Guidelines 01/2025 on pseudonymisation, $EDPB_PAGE.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-eu.sh\` instead.
EOF
write_sha256sums "$EDPB_DIR"
echo "    guidelines.pdf: $edpb_bytes bytes"

echo "Done. Vendored into $DEST"
