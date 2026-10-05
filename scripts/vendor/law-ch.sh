#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the Swiss federal law the compliance pages cite into docs/law/ch/,
# one directory per act, in the format its publisher serves.
#
# WHY THE TEXTS ARE IN THE TREE. Same reason as the EU, Dutch and German
# layers (scripts/vendor/law-{eu,nl,de}.sh): a compliance claim checked
# against a page fetched at review time is checked against whatever that page
# says on the day. A consolidation vendored at a named date is a fixed
# referent, and SHA256SUMS proves the bytes are the ones the script fetched.
#
# FORMAT AND ENDPOINT. The Federal Chancellery publishes the Classified
# Compilation on Fedlex. The web page at www.fedlex.admin.ch/eli/cc/... is a
# single-page application: it answers the same 77 kB shell for every act and
# renders the text in the browser, so it is not the fetch source. The text
# itself is served by the Fedlex filestore at
#   fedlex.data.admin.ch/filestore/fedlex.data.admin.ch/eli/cc/<y>/<n>/<date>/<lang>/html/...
# one HTML document per act, consolidation and language, answering the same
# bytes on every request (measured across repeated fetches when the pins were
# set). That document is vendored verbatim as text-<lang>.html.
#
# THE PIN IS THE CONSOLIDATION DATE. Every act below names the consolidation
# in force when the pin was set (the "Stand am" the document prints); the
# undated ELI would follow Fedlex to a newer state on a re-run, which is what
# a pin exists to prevent. To find the current consolidations, the Fedlex
# SPARQL endpoint (fedlex.data.admin.ch/sparqlendpoint) lists each act's
# jolux:ConsolidationAbstract members with their jolux:dateApplicability and
# the languages each is realised in; move a pin on purpose, review the diff,
# re-read the compliance pages against the new text.
#
# LANGUAGES. German, French and Italian are the authentic languages of Swiss
# federal law; German is vendored as the reference text. Fedlex also publishes
# English translations for some acts and stamps each with "English is not an
# official language of the Swiss Confederation. This translation is provided
# for information purposes only and has no legal force." Where an English
# text exists AT THE SAME CONSOLIDATION as the German pin it is vendored
# beside it, the script asserts the disclaimer is in it, and the record says
# it binds nobody. Where English exists only at an older consolidation (the
# EPDG and EPDV: last translated years before the current state) it is NOT
# vendored — a translation of a superseded text beside the current German one
# would invite citing the wrong version — and the record names the last
# translated consolidation so a reader can fetch it knowingly.
#
# LICENCE. Art. 5 Abs. 1 lit. a URG: laws, ordinances, international treaties
# and other official enactments are not protected by copyright, and Abs. 2
# extends that to official or statutorily required collections and
# translations of them, which is what puts the Fedlex English text under the
# same identifier. Quoted verbatim in
# LICENSES/LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke.txt and declared for
# this subtree in REUSE.toml.
#
# Idempotent: each act directory is wiped and re-fetched.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO_ROOT/docs/law/ch"

# The publisher sees a named client rather than a default curl string, so a
# fetch from this repository is attributable at the far end.
UA='ferroehr-vendor/1.0 (+https://github.com/FerroHEALTH/FerroEHR)'
FETCHED="$(date -u +%Y-%m-%d)"

# directory | ELI path under /eli/cc/ | SR number | consolidation (YYYYMMDD) | languages vendored | byte floor | English at an older consolidation only (YYYYMMDD, or -) | title
#
# The byte floor is roughly two thirds of the German document's size when the
# pin was set — enough to tell an act from an error page.
ACTS=(
  "fadp|2022/491|235.1|20250707|de en|90000|-|Bundesgesetz über den Datenschutz (DSG) — Federal Act on Data Protection (FADP)"
  "dpo|2022/568|235.11|20251201|de en|60000|-|Verordnung über den Datenschutz (DSV) — Ordinance on Data Protection (DPO)"
  "epdg|2017/203|816.1|20241001|de|22000|20200415|Bundesgesetz über das elektronische Patientendossier (EPDG)"
  "epdv|2017/204|816.11|20241001|de|32000|20190401|Verordnung über das elektronische Patientendossier (EPDV)"
  "epdv-edi|2017/205|816.111|20260601|de|65000|-|Verordnung des EDI über das elektronische Patientendossier (EPDV-EDI)"
  "ahvv|63/1185_1183_1185|831.101|20260101|de|380000|-|Verordnung über die Alters- und Hinterlassenenversicherung (AHVV)"
)

# Why each act is here, and which part of it the pages cite.
act_reason() {
  case "$1" in
  fadp) printf '%s' "The Swiss data protection act in force since 1 September 2023, which a Swiss deployment processes under (Switzerland is not an EU member state, so the GDPR is not its law): health data as sensitive personal data (**Art. 5 lit. c**), the processing principles including destruction or anonymisation once the purpose lapses (**Art. 6**), privacy by design and by default (**Art. 7**), data security (**Art. 8**, whose minimum measures the DSV sets), the register of processing activities (**Art. 12**), the data protection impact assessment, mandatory for large-scale processing of sensitive data (**Art. 22**), breach notification (**Art. 24**), the right of access including delivery of health data through a designated health professional (**Art. 25**), data portability in a common electronic format (**Art. 28**) and the research ground with its anonymisation duty (**Art. 31 Abs. 2 lit. e**)." ;;
  dpo) printf '%s' "The ordinance under the DSG carrying the minimum data security measures Art. 8 Abs. 3 DSG delegates: the four goals of confidentiality, availability, integrity and traceability (**Art. 2**), the named controls from access control to disclosure control and breach detection (**Art. 3**), the logging duty for large-scale processing of sensitive data — storing, changing, disclosing, deleting, destroying AND reading, each entry with actor, kind, date, time and recipient, kept at least one year separately from the processing system and readable only by the oversight roles (**Art. 4**, as amended on 1 December 2025), the processing regulations (**Art. 5 and 6**), the modalities of the right of access (**Art. 16 to 19**) and of portability (**Art. 20 to 22**), and the removal of the small-organisation exemption from the register for large-scale sensitive data (**Art. 24**)." ;;
  epdg) printf '%s' "The act behind the Swiss electronic patient record (EPD), a retrieval layer over records that stay where they are (**Art. 2 lit. a**): opt-in consent (**Art. 3**), the patient identification number issued by the ZAS (**Art. 4 to 6**), the patient\x27s control of access rights and confidentiality levels including emergency access (**Art. 9**), the duties of certified communities — log every processing and keep the logs ten years (**Art. 10**) — and their certification (**Art. 11 to 13**), and the penalty for accessing an EPD without a right (**Art. 24**). A CDR is not a community; the act states what a provider\x27s system must be able to do when it feeds one. The duty of hospitals to join a certified community is enacted through **Art. 25**, an amendment of the KVG whose text Fedlex publishes only by reference (AS 2017 2201); this corpus does not carry it." ;;
  epdv) printf '%s' "The Federal Council\x27s ordinance under the EPDG: the three confidentiality levels, the two grantable access rights, emergency access with notification of the patient, time-limited group rights and delegation without escalation (**Art. 1 to 4**), the patient identification number — its structure, its issue by the ZAS, manual entry only with a check-digit control, no reuse after annulment (**Art. 5 to 8**), the duties of communities — separate storage of EPD data, state-of-the-art encryption for storage and transmission, destruction after twenty years, metadata, exchange formats, integration profiles and log-data specifications delegated to the EDI (**Art. 10**), a risk-based data protection and security management system with data stores in Switzerland (**Art. 12**) —, the patient portal\x27s display of the log data (**Art. 18**), pseudonymised evaluation data for the BAG (**Art. 22**), and the certification of communities against Art. 9 to 21 (**Art. 30**)." ;;
  epdv-edi) printf '%s' "The departmental ordinance under the EPDV: the patient identification number\x27s structure and check digit (**Art. 1 and Anhang 1**, published in full — eighteen digits, a mod-10 check), and the integration profiles a community and any system feeding one are measured against (**Art. 5 and Anhang 5**, published in full — IHE ATNA with Record Audit Event ITI-20 and Authenticate Node ITI-19, CH:ATC with Retrieve ATNA Audit Event ITI-81, IUA, XUA, XDS, MHD, PIXm, PDQm and the national profiles). The certification criteria, the metadata, the exchange formats and the directory metadata (**Anhänge 2, 3, 4, 8 and 9**) are published by reference only and are NOT in this corpus; a claim about their content cannot be grounded here." ;;
  ahvv) printf '%s' "The ordinance under the AHVG, vendored for one identifier: **Art. 133** defines the thirteen-digit AHV number (the country code 756, nine digits that allow no inference about the person, a check digit), **Art. 133bis** puts its assignment with the ZAS, and **Art. 134quinquies Abs. 2** requires a check-digit control before a manual entry. The check-digit arithmetic itself is not in the ordinance; it is in the BSV's Wegleitung über Versicherungsausweis und individuelles Konto (WL VA/IK, 318.106.02 d, Anhang 7), which the identifier scanner's \`ch-ahvn13\` rule cites." ;;
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

# $1 url, $2 destination, $3 byte floor, $4 a phrase the real document must
# contain (empty to skip). Prints the byte count; refuses everything else.
fetch() {
  local url="$1" out="$2" floor="$3" needle="$4" code bytes
  code="$(curl -sS -A "$UA" -L --proto '=https' --proto-redir '=https' -o "$out" -w '%{http_code}' "$url")"
  if [[ "$code" != 200 ]]; then
    echo "ERROR: $url answered HTTP $code" >&2
    exit 1
  fi
  bytes="$(wc -c <"$out" | tr -d ' ')"
  if [[ "$bytes" -lt "$floor" ]]; then
    echo "ERROR: $url returned $bytes bytes, under the $floor-byte floor — that is an error page, not an act" >&2
    exit 1
  fi
  if [[ -n "$needle" ]] && ! grep -q -- "$needle" "$out"; then
    echo "ERROR: $url returned $bytes bytes that never say '$needle' — that is not the document expected" >&2
    exit 1
  fi
  printf '%s' "$bytes"
}

# The filestore address of one act text: ELI path, consolidation, language.
filestore_url() {
  local eli="$1" date="$2" lang="$3" dashed
  dashed="${eli//\//-}"
  printf 'https://fedlex.data.admin.ch/filestore/fedlex.data.admin.ch/eli/cc/%s/%s/%s/html/fedlex-data-admin-ch-eli-cc-%s-%s-%s-html.html' \
    "$eli" "$date" "$lang" "$dashed" "$date" "$lang"
}

# The heading block Fedlex prints at the top of a document, with the markup
# taken off: the title (which the publisher breaks with <br>), the short
# title with the abbreviation, and the "vom … (Stand am …)" line.
heading_field() {
  local file="$1" class="$2" tag="$3"
  grep -o "<$tag class=\"${class}[^\"]*\">[^<]*\(<br>[^<]*\)*" "$file" | head -n 1 |
    sed -e 's/<[^>]*>/ /g' -e 's/  */ /g' -e 's/^ //' -e 's/ $//'
}

EN_DISCLAIMER='not an official language of the Swiss Confederation'

mkdir -p "$DEST"

for entry in "${ACTS[@]}"; do
  IFS='|' read -r dir eli sr consolidated langs floor older_en title <<<"$entry"
  out="$DEST/$dir"
  echo "==> ch/$dir (SR $sr, /eli/cc/$eli @ $consolidated)"
  rm -rf "$out"
  mkdir -p "$out"

  files_rows=""
  for lang in $langs; do
    url="$(filestore_url "$eli" "$consolidated" "$lang")"
    file="text-$lang.html"
    if [[ "$lang" == en ]]; then
      bytes="$(fetch "$url" "$out/$file" "$floor" "$EN_DISCLAIMER")"
    else
      bytes="$(fetch "$url" "$out/$file" "$floor" "Art.")"
    fi
    digest="$(sha256_of "$out/$file")"
    files_rows="${files_rows}| \`${file}\` | ${bytes} | \`${digest}\` | ${url} |
"
    echo "    $file: $bytes bytes"
  done

  de="$out/text-de.html"
  full_title="$(heading_field "$de" erlasstitel h1)"
  short_title="$(heading_field "$de" erlasskurztitel h2)"
  dated="$(heading_field "$de" erlassdatum p)"
  page_dated="https://www.fedlex.admin.ch/eli/cc/$eli/$consolidated/de"
  page_undated="https://www.fedlex.admin.ch/eli/cc/$eli/de"

  if [[ "$langs" == *en* ]]; then
    english_note="**The English text binds nobody.** Fedlex stamps it: \"English is not
an official language of the Swiss Confederation. This translation is provided
for information purposes only and has no legal force.\" It is vendored at the
same consolidation as the German text, and the script refuses an English file
that does not carry that disclaimer. Cite the German; read the English."
  elif [[ "$older_en" != "-" ]]; then
    english_note="**No English text is vendored.** Fedlex last published an English
translation of this act at the consolidation of $older_en, an older state than
the German text here; a translation of a superseded text beside the current
one would invite citing the wrong version. A reader who wants it knowingly
finds it at https://www.fedlex.admin.ch/eli/cc/$eli/$older_en/en (non-binding,
by Fedlex's own disclaimer)."
  else
    english_note="**No English text exists** for this act on Fedlex; the German text is
the one vendored, and French and Italian are equally authentic at
https://www.fedlex.admin.ch/eli/cc/$eli/$consolidated/fr and
https://www.fedlex.admin.ch/eli/cc/$eli/$consolidated/it."
  fi

  cat >"$out/PROVENANCE.md" <<EOF
# $title

$(act_reason "$dir")

| | |
|---|---|
| Title (German, authentic) | $full_title $short_title |
| Enacted / state | $dated |
| SR number | \`$sr\` |
| ELI | https://fedlex.data.admin.ch/eli/cc/$eli |
| Consolidation vendored | $consolidated (Stand am) |
| Human-readable at | $page_dated (this consolidation), $page_undated (whatever is current) |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-ch.sh\` |

## Files

| file | bytes | SHA-256 | fetched from |
|---|---|---|---|
$files_rows
The HTML the Fedlex filestore serves for this consolidation and language,
byte for byte. Nothing was converted or extracted: an extraction is an edit,
and an edited act is no longer the publisher's text. The www.fedlex.admin.ch
page above renders the same document but is a single-page application that
answers an identical shell for every act, which is why it is not the fetch
source.

$english_note

## Licence

Not protected by copyright. Art. 5 Abs. 1 lit. a URG: *"Durch das
Urheberrecht nicht geschützt sind: a. Gesetze, Verordnungen, völkerrechtliche
Verträge und andere amtliche Erlasse"*, and Abs. 2: *"Ebenfalls nicht
geschützt sind amtliche oder gesetzlich geforderte Sammlungen und
Übersetzungen der Werke nach Absatz 1."* The article is quoted in
\`LICENSES/LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke.txt\` and declared for
this subtree in \`REUSE.toml\`.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-ch.sh\` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.
EOF

  write_sha256sums "$out"
done

echo "Done. Vendored into $DEST"
