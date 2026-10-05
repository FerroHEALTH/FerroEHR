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
# FILES BESIDE THE PINNED TEXT. A consolidation carries the enacting terms and
# no recitals; EUR-Lex's own header on it says the authentic versions,
# "including their preambles", are the ones in the Official Journal. Where the
# pages need recitals, or a corrigendum or amending act the reader has to see,
# that OJ document is fetched beside the pinned text from its own CELEX (the
# EXTRAS table below) and the act's PROVENANCE.md says which file to cite for
# what.
#
# Idempotent: each act directory is wiped and re-fetched. Re-run after changing
# a pin below; the SHA-256 lines move with the bytes, and docs/law/README.md is
# the index that must move with them. With no argument every act is vendored;
# with arguments, only the named directories, so adding one act does not
# re-stamp the fetch date of every other record:
#
#   scripts/vendor/law-eu.sh                 # the whole EU layer
#   scripts/vendor/law-eu.sh eprivacy gdpr   # those two directories only
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
  "eprivacy|02002L0058-20091219|http://data.europa.eu/eli/dir/2002/58/oj|2009-12-19|45000|Directive 2002/58/EC — privacy and electronic communications (ePrivacy Directive)"
)

# directory | file | CELEX (percent-encoded) | media type | byte floor | a word the document must contain
#
# OJ documents vendored beside an act's pinned text. The media type is the one
# the Publications Office serves for that CELEX: the 2002 Directive predates
# the XHTML manifestations and is served as HTML or PDF only, and the HTML is
# taken because it is text a recital citation can land on.
EXTRAS=(
  "gdpr|oj.html|32016R0679|application/xhtml+xml|540000|Whereas"
  "gdpr|corrigendum-2018-05-23.html|32016R0679R%2802%29|application/xhtml+xml|15000|Corrigendum"
  "eprivacy|oj.html|32002L0058|text/html|40000|Whereas"
  "eprivacy|amending-directive-2009-136.html|32009L0136|application/xhtml+xml|190000|2002/58/EC"
)

EDPB_NAME="edpb-guidelines-01-2025-pseudonymisation"

# The directories named on the command line, each checked against the tables
# so a typo stops the run instead of vendoring nothing.
ONLY=("$@")
for want in "${ONLY[@]+"${ONLY[@]}"}"; do
  known=0
  for entry in "${ACTS[@]}"; do
    [[ "${entry%%|*}" == "$want" ]] && known=1
  done
  [[ "$want" == "$EDPB_NAME" ]] && known=1
  if [[ "$known" -eq 0 ]]; then
    echo "ERROR: '$want' is not a directory this script vendors" >&2
    exit 1
  fi
done

selected() {
  local want
  [[ ${#ONLY[@]} -eq 0 ]] && return 0
  for want in "${ONLY[@]}"; do
    [[ "$want" == "$1" ]] && return 0
  done
  return 1
}

# Why each act is here, in one sentence, written into its PROVENANCE.md.
act_reason() {
  case "$1" in
  gdpr) printf '%s' "The processing law every deployment answers to: the lawfulness, minimisation, security and rights obligations the pseudonymisation boundary, the access log and the records of processing are built against." ;;
  ehds) printf '%s' "The health-data regulation whose Chapter III puts requirements on an EHR system itself, including the Annex II logging elements the audit trail is measured against." ;;
  nis2) printf '%s' "The cybersecurity directive that binds the essential and important entities a deployment of this software typically is, and the incident-reporting regime around it." ;;
  cra) printf '%s' "The horizontal cybersecurity regulation for products with digital elements, which EHDS Chapter III cross-references for the essential requirements an EHR system inherits." ;;
  mdr) printf '%s' "The medical-device regulation behind the Article 27 interoperability question the EHDS readiness page leaves open for a deployment that claims interoperability with a device." ;;
  eprivacy) printf '%s' "The directive whose Article 5(3) governs storing information in, or gaining access to information stored in, the terminal equipment of a subscriber or user, which the usage report's default is read against (#3580)." ;;
  *) printf '%s' "" ;;
  esac
}

# What each file beside the pinned text is, one sentence, for the Files table.
extra_role() {
  case "$1/$2" in
  gdpr/oj.html) printf '%s' "The Regulation as published in OJ L 119, 4.5.2016, p. 1, recitals included." ;;
  gdpr/corrigendum-2018-05-23.html) printf '%s' "The English corrigendum, OJ L 127, 23.5.2018, p. 2." ;;
  eprivacy/oj.html) printf '%s' "The Directive as published in OJ L 201, 31.7.2002, p. 37, recitals included." ;;
  eprivacy/amending-directive-2009-136.html) printf '%s' "Directive 2009/136/EC as published in OJ L 337, 18.12.2009, p. 11, whole." ;;
  *) printf '%s' "" ;;
  esac
}

# A section of the record that only some acts need: which file to cite for
# what, and what the pinned text does not carry. Written from the vendored
# files and the EUR-Lex document metadata as read on the date it names.
act_note() {
  case "$1" in
  gdpr)
    cat <<'EOF'
## Recitals, the OJ text and the corrigendum

`text.html` is the consolidation and carries the enacting terms only: it has
no recital and no `id="rct_…"` anchor. Its own header gives the reason: "The
authentic versions of the relevant acts, including their preambles, are those
published in the Official Journal of the European Union". The recitals are
therefore vendored from the OJ text beside it.

- **Recitals** are cited from `oj.html`, the Regulation as published in OJ L
  119, 4.5.2016 (anchors `id="rct_1"` to `id="rct_173"`):
  `docs/law/eu/gdpr/oj.html Recital 63`.
- **Articles** are cited from `text.html`, as before:
  `docs/law/eu/gdpr/text.html Art. 15(1)`. The consolidation folds in the
  English corrigendum of 23 May 2018 (marked ►C1 in it); `oj.html` does not,
  so the article wording in `oj.html` is the uncorrected 2016 text and is not
  the citation referent.
- `corrigendum-2018-05-23.html` is that corrigendum. It corrects recital 71
  (fifth and sixth sentences) and points of Articles 37(1), 41(3), 41(5),
  42(7), 43(3), 43(6), 57(1), 64(1), 64(6) to (8), 65(1), 69(2) and 70(1). A
  citation of recital 71 reads `oj.html` together with it.

Both texts are kept: the consolidation for the articles, the OJ text for the
recitals. Replacing the consolidation with the OJ text would move every
existing article citation onto the uncorrected 2016 wording (#3581).
EOF
    ;;
  eprivacy)
    cat <<'EOF'
## What each file is for

`text.html` is the consolidation at 19 December 2009. It folds in Directive
2006/24/EC (marked ►M1 in it) and Directive 2009/136/EC (►M2), so Article
5(3) in it is the wording Directive 2009/136/EC substituted. It carries the
articles only: no recital is in it. Cite the articles from here:
`docs/law/eu/eprivacy/text.html Art. 5(3)`.

- `oj.html` is the Directive as published in OJ L 201, 31.7.2002, with its
  recitals. The Publications Office serves this act as HTML in the older
  EUR-Lex layout or as PDF, and no XHTML, so the HTML is vendored. Its
  articles are the 2002 wording, before either amendment; cite the recitals of
  Directive 2002/58/EC from here.
- `amending-directive-2009-136.html` is Directive 2009/136/EC as published in
  OJ L 337, 18.12.2009, vendored whole: its recitals, its Article 2 amending
  Directive 2002/58/EC, and its amendments to the other acts it touches. The
  recitals that accompany the 2009 wording of Article 5(3), recital 66 among
  them, are in this file; `text.html` carries none of them.

## Derogations the text does not show

The EUR-Lex document metadata for CELEX `32002L0058`, read 2026-10-05, lists
no amendment after Directive 2009/136/EC. It lists two regulations that
derogate from Articles 5(1) and 6(1) without changing the text: Regulation
(EU) 2021/1232 (from 2 August 2021 to 3 August 2024) and Regulation (EU)
2026/1881. Neither is vendored, and the metadata names neither as touching
Article 5(3).
EOF
    ;;
  cra)
    cat <<'EOF'
## An amendment this text does not carry

Regulation (EU) 2025/327 (EHDS), Article 104, amends this Regulation. It
replaces Article 13(4) and Article 31(3) and inserts Article 32(5a), under
which manufacturers of products with digital elements classified as EHR
systems under the EHDS "shall demonstrate conformity with the essential
requirements set out in Annex I to this Regulation using the relevant
conformity assessment procedure provided for in Chapter III of Regulation (EU)
2025/327". EHDS Article 105 makes the EHDS apply from 26 March 2027 and does
not list Article 104 among the provisions with a later date.

`text.html` is the OJ text of 20 November 2024 and carries none of the three
changes. A reader of Article 13(4), 31(3) or 32 for an EHR system reads the
amended wording in `docs/law/eu/ehds/text.html` Art. 104 (`id="art_104"`).

EUR-Lex publishes no consolidation that folds the amendment in. On 2026-10-05
the only consolidated version it lists for this act is `02024R2847-20241120`,
the initial one, and its document metadata gives 26 March 2027 as the date
from which the three changes apply. The OJ text therefore stays pinned. The
metadata also lists one English corrigendum, CELEX `32024R2847R(01)` (OJ L,
2024/90780, 5.12.2024), which corrects the reference to Regulation (EU)
2019/1020 in the title and nothing else; it is not vendored.
EOF
    ;;
  *) ;;
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
# $1 the CELEX URI, $2 the media type to negotiate.
resolve_celex() {
  local uri="$1" accept="$2" location
  location="$(curl -sS -A "$UA" --proto '=https' -D - -o /dev/null \
    -H "Accept: $accept" -H 'Accept-Language: eng' "$uri" \
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
  selected "$dir" || continue
  celex_uri="https://publications.europa.eu/resource/celex/$celex"
  url="$(resolve_celex "$celex_uri" "application/xhtml+xml")"
  page="https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:$celex"
  out="$DEST/$dir"
  echo "==> eu/$dir (CELEX:$celex)"
  rm -rf "$out"
  mkdir -p "$out"

  bytes="$(fetch "$url" "$out/text.html" "$floor" "Article" "application/xhtml+xml")"
  digest="$(sha256_of "$out/text.html")"

  # The OJ documents beside the pinned text, each from its own CELEX.
  extra_rows=""
  extra_sources=""
  extra_names=""
  for extra in "${EXTRAS[@]}"; do
    IFS='|' read -r xdir xfile xcelex xaccept xfloor xneedle <<<"$extra"
    [[ "$xdir" == "$dir" ]] || continue
    xuri="https://publications.europa.eu/resource/celex/$xcelex"
    xurl="$(resolve_celex "$xuri" "$xaccept")"
    xbytes="$(fetch "$xurl" "$out/$xfile" "$xfloor" "$xneedle" "$xaccept")"
    xdigest="$(sha256_of "$out/$xfile")"
    extra_rows+="| \`$xfile\` | $xbytes | \`$xdigest\` |"$'\n'
    extra_sources+="| \`$xfile\` | $(extra_role "$dir" "$xfile") | $xuri (\`Accept: $xaccept\`) | $xurl |"$'\n'
    extra_names+="${extra_names:+ and }\`$xfile\`"
    echo "    $xfile: $xbytes bytes"
  done

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
    if [[ -n "$extra_names" ]]; then
      licence_line+="
  The CC BY 4.0 layer covers \`text.html\` only. The other files,
  $extra_names,
  are OJ documents, not consolidated texts, so they come under the reuse
  policy alone, and \`REUSE.toml\` declares them that way."
    fi
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

  # Optional sections, each carrying its own surrounding blank lines so an act
  # without them gets no stray empty lines in its record.
  extra_block=""
  if [[ -n "$extra_sources" ]]; then
    extra_block=$'\n'"The other files are OJ documents fetched the same way from their own
CELEX, byte for byte:

| file | what it is | fetched from | resolved to |
|---|---|---|---|
${extra_sources}"
  fi
  note_block="$(act_note "$dir")"
  [[ -z "$note_block" ]] || note_block=$'\n'"$note_block"$'\n'

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
${extra_rows}
\`text.html\` is the English XHTML the Publications Office serves for this
CELEX, byte for byte. Nothing was converted or extracted. The same document is
what the EUR-Lex page above renders; that page is not the fetch source because
it wraps the act in site chrome carrying a per-request identifier, which would
make the digest above unreproducible.
${extra_block}
## Version

$version_note
${note_block}
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
if ! selected "$EDPB_NAME"; then
  echo "Done. Vendored into $DEST"
  exit 0
fi
EDPB_DIR="$DEST/$EDPB_NAME"
EDPB_URL="https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf"
EDPB_PAGE="https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en"
echo "==> eu/$EDPB_NAME"
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
