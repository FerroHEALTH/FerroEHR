#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the German federal law the compliance pages cite into docs/law/de/,
# one directory per act, in the format its publisher serves.
#
# WHY THE TEXTS ARE IN THE TREE. Same reason as the EU and Dutch layers
# (scripts/vendor/law-eu.sh, scripts/vendor/law-nl.sh): a compliance claim
# checked against a page fetched at review time is checked against whatever
# that page says on the day. A text vendored at a named state is a fixed
# referent, and SHA256SUMS proves the bytes are the ones the script fetched.
#
# FORMAT AND SOURCE. The consolidated federal law is published by the
# Bundesministerium der Justiz and the Bundesamt für Justiz at
# gesetze-im-internet.de, which serves every act as one XML document (the
# gii-norm DTD) inside <slug>/xml.zip. The XML is vendored as the file the zip
# carries, under the publisher's own file name; the zip is a container, and
# unpacking it is not an edit of the text. The zip's own digest is recorded
# in the provenance beside the XML's.
#
# THE PIN IS THE BUILD DATE, NOT A URL. gesetze-im-internet.de has no dated
# URL: xml.zip is always the current state of the act. What the publisher does
# stamp is a builddate on the document root. Each act below is pinned at the
# builddate the vendored file carries, and this script REFUSES a fetch whose
# builddate differs from the pin, so an upstream change can never slide into
# the tree under a re-run: the pin is moved on purpose, the diff is reviewed,
# and the compliance pages are re-read against the new text.
#
# THE CONSOLIDATION IS NOT THE AUTHENTIC TEXT, and the publisher says so: the
# "Hinweise" page describes the consolidated texts as "nicht amtliche Texte".
# The authentic text is the Bundesgesetzblatt. What a compliance claim is read
# against is what the act says today, which is the consolidation; a citation
# that must be authoritative goes to the BGBl. issue. The DigiG is vendored
# from there directly, because it is an amending act: its substance lives in
# the acts it amended (SGB V above all) and gesetze-im-internet.de carries no
# consolidated text of it, so the promulgated issue on recht.bund.de is the
# only text there is.
#
# WHOLE ACTS ONLY. § 203 StGB is one section of the criminal code and the
# ePA provisions are a chapter of SGB V, but the publisher serves each act as
# one document and cutting the cited sections out of it would be an edit. The
# whole act is vendored and the provenance names the sections the pages cite.
#
# LICENCE. § 5 Abs. 1 UrhG: laws, ordinances, official decrees and notices
# enjoy no copyright protection. The paragraph is quoted verbatim in
# LICENSES/LicenseRef-UrhG-Para5-Amtliche-Werke.txt and declared for this
# subtree in REUSE.toml.
#
# OUT OF SCOPE, deliberately: the sixteen Landeskrankenhausgesetze and the
# Landesdatenschutzgesetze. A hospital reads its own state's law beside the
# federal layer here; this corpus carries federal law only, and the README
# says so.
#
# Idempotent: each act directory is wiped and re-fetched.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO_ROOT/docs/law/de"

# The publisher sees a named client rather than a default curl string, so a
# fetch from this repository is attributable at the far end.
UA='ferroehr-vendor/1.0 (+https://github.com/FerroHEALTH/FerroEHR)'
FETCHED="$(date -u +%Y-%m-%d)"

# directory | gesetze-im-internet slug | pinned builddate | byte floor of the XML | title
#
# The byte floor is roughly two thirds of the size measured when the pin was
# set — enough to tell an act from the error page a publisher answers with
# when something goes wrong.
ACTS=(
  "bdsg|bdsg_2018|20260713215512|150000|Bundesdatenschutzgesetz (BDSG)"
  "sgb-v|sgb_5|20260810220008|2000000|Sozialgesetzbuch Fünftes Buch (SGB V) — Gesetzliche Krankenversicherung"
  "gdng|gdng|20240425215012|20000|Gesundheitsdatennutzungsgesetz (GDNG)"
  "stgb|stgb|20260702215503|550000|Strafgesetzbuch (StGB)"
)

# Why each act is here, and which part of it the pages cite.
act_reason() {
  case "$1" in
  bdsg) printf '%s' "The German act beside the GDPR, of which Part 2 (§§ 22 to 44) is what a care provider processes under: **§ 22** (the health-care ground for special categories in Abs. 1 Nr. 1 lit. b, and the measures Abs. 2 names — after-the-fact traceability of who entered, changed or removed data, access restriction inside the controller and its processors, pseudonymisation, encryption), **§ 27** (research: identifying characteristics stored separately and rejoined only as the research purpose requires, anonymised as soon as possible) and **§ 35** (restriction of processing in place of erasure). Part 3 (§§ 45 ff., among them the § 64 security catalogue and the § 76 logging rule) binds the law-enforcement bodies of Directive (EU) 2016/680 only, per § 45, and is not cited against a hospital." ;;
  sgb-v) printf '%s' "The statutory health insurance code, whose **§§ 341 to 355** define the elektronische Patientenakte (ePA) and its access rules, and whose **§§ 360 to 363** govern the Telematikinfrastruktur, as amended by the Patientendaten-Schutz-Gesetz (2020) and the Digital-Gesetz (2024). A CDR is not an ePA and not a TI component; these sections state what a care provider's own system must be able to do beside them. The whole book is vendored, because the publisher serves it as one document." ;;
  gdng) printf '%s' "The health-data-use act for secondary use: the data access and coordination body at the BfArM (**§ 3**), the linkage of research-data-centre data with the cancer registries through a secure processing environment (**§ 4**), the lead supervisory authority for multi-state research (**§ 5**), and the one section that reaches a care provider\x27s own system, **§ 6**: a provider may reprocess its own patient data for quality assurance, research and statistics if it pseudonymises, anonymises as soon as possible, runs a rights-and-roles concept with logging, deletes after thirty years at the latest, and can tell a data subject the kind, scope and purpose of that processing. **§§ 7 to 9** bind the data users (secrecy, registration and publication, penalties)." ;;
  stgb) printf '%s' "The criminal code, for one section: **§ 203**, the professional secrecy of physicians and their staff, which every access to a clinical record by a person outside the treatment context is measured against, and whose Abs. 3 and 4 (since 2017) govern disclosure to the persons and contractors who keep the systems running. The whole code is vendored, because the publisher serves it as one document." ;;
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

# $1 url, $2 destination, $3 byte floor, $4 the leading bytes the real
# document must start with (empty to skip). Prints the byte count; refuses
# everything else.
fetch() {
  local url="$1" out="$2" floor="$3" magic="$4" code bytes
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
  if [[ -n "$magic" ]] && [[ "$(head -c "${#magic}" "$out")" != "$magic" ]]; then
    echo "ERROR: $url did not answer with a document starting '$magic'" >&2
    exit 1
  fi
  printf '%s' "$bytes"
}

# The stand lines the publisher records on the document: the amendment the
# consolidation last folded in, and any notice that a later amendment is not
# yet fully worked in. Printed one per line as "type: comment".
stand_lines() {
  grep -o '<standangabe[^>]*><standtyp>[^<]*</standtyp><standkommentar>[^<]*' "$1" |
    sed -e 's/<standangabe[^>]*><standtyp>//' -e 's|</standtyp><standkommentar>|: |' || true
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$DEST"

for entry in "${ACTS[@]}"; do
  IFS='|' read -r dir slug pinned floor title <<<"$entry"
  url="https://www.gesetze-im-internet.de/$slug/xml.zip"
  page="https://www.gesetze-im-internet.de/$slug/"
  out="$DEST/$dir"
  echo "==> de/$dir ($slug, builddate $pinned)"

  zip="$WORK/$slug.zip"
  zip_bytes="$(fetch "$url" "$zip" 5000 'PK')"
  zip_digest="$(sha256_of "$zip")"

  # One XML per act is what the publisher ships today; the loop below vendors
  # whatever the zip carries so an act that gains an image is not truncated.
  unpack="$WORK/$slug"
  rm -rf "$unpack"
  mkdir -p "$unpack"
  unzip -q -o "$zip" -d "$unpack"
  xml="$(find "$unpack" -maxdepth 1 -type f -name '*.xml' | head -n 1)"
  if [[ -z "$xml" ]]; then
    echo "ERROR: $url carried no XML document" >&2
    exit 1
  fi
  xml_name="$(basename "$xml")"
  xml_bytes="$(wc -c <"$xml" | tr -d ' ')"
  if [[ "$xml_bytes" -lt "$floor" ]]; then
    echo "ERROR: $xml_name is $xml_bytes bytes, under the $floor-byte floor — that is not the act" >&2
    exit 1
  fi
  if ! grep -q '<enbez>§' "$xml"; then
    echo "ERROR: $xml_name never numbers a section — that is not an act text" >&2
    exit 1
  fi

  builddate="$(grep -o '<dokumente builddate="[0-9]*"' "$xml" | head -n 1 | sed 's/[^0-9]//g')"
  if [[ "$builddate" != "$pinned" ]]; then
    echo "ERROR: $slug is now built $builddate, the pin says $pinned. gesetze-im-internet.de has moved the act; update the pin in this script ON PURPOSE, then re-read the compliance pages against the new text" >&2
    exit 1
  fi
  doknr="$(grep -o '<dokumente builddate="[0-9]*" doknr="[^"]*"' "$xml" | head -n 1 | sed 's/.*doknr="//; s/"//')"
  jurabk="$(grep -o '<jurabk>[^<]*' "$xml" | head -n 1 | sed 's/<jurabk>//')"
  langue="$(grep -o '<langue>[^<]*' "$xml" | head -n 1 | sed 's/<langue>//')"
  ausfertigung="$(grep -o '<ausfertigung-datum[^>]*>[^<]*' "$xml" | head -n 1 | sed 's/<[^>]*>//')"
  fundstelle="$(grep -o '<fundstelle typ="amtlich"><periodikum>[^<]*</periodikum><zitstelle>[^<]*' "$xml" | head -n 1 | sed -e 's/<fundstelle typ="amtlich"><periodikum>//' -e 's|</periodikum><zitstelle>| |')"
  stand="$(stand_lines "$xml" | sed 's/^/| /; s/$/ |/; s/: / | /')"
  if [[ -z "$stand" ]]; then
    stand="| — | The publisher records no stand line: the act has not been amended since its promulgation |"
  fi

  rm -rf "$out"
  mkdir -p "$out"
  cp "$unpack"/* "$out/"
  xml_digest="$(sha256_of "$out/$xml_name")"

  cat >"$out/PROVENANCE.md" <<EOF
# $title

$(act_reason "$dir")

| | |
|---|---|
| Official title | $langue |
| Abbreviation (juris) | \`$jurabk\` |
| Document number | \`$doknr\` |
| Promulgated | $ausfertigung, $fundstelle |
| Consolidation vendored | builddate \`$builddate\` (the publisher's build stamp; see the stand lines below) |
| Source | $url |
| Human-readable at | $page |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-de.sh\` |

## Stand

The publisher's own statement of which amendment the consolidation last
folded in, and of any amendment already promulgated but not yet fully worked
into the text:

| type | statement |
|---|---|
$stand

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`$xml_name\` | $xml_bytes | \`$xml_digest\` |

The XML gesetze-im-internet.de serves for this act, byte for byte, under the
publisher's own file name, unpacked from \`xml.zip\` (the zip fetched was
$zip_bytes bytes, SHA-256 \`$zip_digest\`). Nothing was converted or
extracted: an extraction is an edit, and an edited act is no longer the
publisher's text.

**This is the consolidation, not the authentic text.** gesetze-im-internet.de
describes its consolidated texts as non-official ("nicht amtliche Texte"); the
authentic text is the Bundesgesetzblatt. The consolidation is what the act
says today, which is what a compliance claim is read against; a citation that
must be authoritative goes to the BGBl. issue.

**There is no dated URL for this act.** The publisher serves only the current
state, so the pin is the \`builddate\` above and the vendor script refuses a
fetch whose build stamp differs from it. Moving the pin is a deliberate act.

## Licence

No copyright subsists in this text. § 5 Abs. 1 UrhG: *"Gesetze, Verordnungen,
amtliche Erlasse und Bekanntmachungen sowie Entscheidungen und amtlich
verfaßte Leitsätze zu Entscheidungen genießen keinen urheberrechtlichen
Schutz."* The paragraph is quoted in
\`LICENSES/LicenseRef-UrhG-Para5-Amtliche-Werke.txt\` and declared for this
subtree in \`REUSE.toml\`.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-de.sh\` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.
EOF

  write_sha256sums "$out"
  echo "    $xml_name: $xml_bytes bytes (zip $zip_bytes bytes)"
done

# ── The DigiG: the promulgated issue, because there is no consolidation ─────
# The Digital-Gesetz is an Artikelgesetz: it amends SGB V and its neighbours
# and has next to no free-standing text of its own, so gesetze-im-internet.de
# carries no consolidated act for it. The authentic text is the issue of the
# Bundesgesetzblatt that promulgated it, served by the federal promulgation
# platform recht.bund.de as PDF (the platform publishes the BGBl. in PDF only).
DIGIG_DIR="$DEST/digig"
DIGIG_URL='https://www.recht.bund.de/bgbl/1/2024/101/regelungstext.pdf?__blob=publicationFile&v=1'
DIGIG_PAGE='https://www.recht.bund.de/bgbl/1/2024/101/VO.html'
DIGIG_ELI='https://recht.bund.de/eli/bund/bgbl-1/2024/101'
echo "==> de/digig (BGBl. 2024 I Nr. 101)"
rm -rf "$DIGIG_DIR"
mkdir -p "$DIGIG_DIR"
digig_bytes="$(fetch "$DIGIG_URL" "$DIGIG_DIR/regelungstext.pdf" 650000 '%PDF-')"
digig_digest="$(sha256_of "$DIGIG_DIR/regelungstext.pdf")"
cat >"$DIGIG_DIR/PROVENANCE.md" <<EOF
# Gesetz zur Beschleunigung der Digitalisierung des Gesundheitswesens (Digital-Gesetz, DigiG)

The 2024 act that rewrote the ePA into an opt-out record and re-cut the
Telematikinfrastruktur provisions of SGB V. It is an **amending act**: its
substance is the new wording of **SGB V §§ 341 to 355 and 360 to 363**, which
the consolidated SGB V in \`../sgb-v/\` already carries. The issue is vendored
because it is the authentic text of those amendments, with its own entry into
force provisions (**Art. 9**: the day after promulgation, with the opt-out ePA
provisions on 15 January 2025), which no consolidation shows.

| | |
|---|---|
| Act dated | 22 March 2024 |
| Promulgated | BGBl. 2024 I Nr. 101 — the platform dates the issue 25 March 2024, the PDF masthead reads "Ausgegeben zu Bonn am 26. März 2024"; a correction followed in BGBl. 2024 I Nr. 101a |
| ELI | $DIGIG_ELI |
| Source | $DIGIG_URL |
| Human-readable at | $DIGIG_PAGE |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-de.sh\` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`regelungstext.pdf\` | $digig_bytes | \`$digig_digest\` |

**No text format is published.** The federal promulgation platform issues the
Bundesgesetzblatt in PDF and offers no HTML or XML edition of an issue, so the
PDF is vendored as published and not converted: a converted copy would be
this repository's rendering, and a page cited against it would resolve to our
pagination rather than the publisher's. The platform answers the same bytes
on every request (measured across repeated fetches when the pin was set).

**Why there is no consolidated DigiG.** gesetze-im-internet.de consolidates
Stammgesetze, the acts that amendments are folded into. An Artikelgesetz
disappears into them: after consolidation the DigiG's text IS the current
text of SGB V and the other acts it touched. Read the ePA and TI provisions
in \`../sgb-v/\`; read this issue for what changed, when, and from which date.

## Licence

No copyright subsists in this text. § 5 Abs. 1 UrhG: *"Gesetze, Verordnungen,
amtliche Erlasse und Bekanntmachungen sowie Entscheidungen und amtlich
verfaßte Leitsätze zu Entscheidungen genießen keinen urheberrechtlichen
Schutz."* The paragraph is quoted in
\`LICENSES/LicenseRef-UrhG-Para5-Amtliche-Werke.txt\` and declared for this
subtree in \`REUSE.toml\`.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-de.sh\` instead.
EOF
write_sha256sums "$DIGIG_DIR"
echo "    regelungstext.pdf: $digig_bytes bytes"

echo "Done. Vendored into $DEST"
