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
  "myhealth-eu-2026-2083|32026R2083|http://data.europa.eu/eli/reg_impl/2026/2083/oj||90000|Commission Implementing Regulation (EU) 2026/2083 — MyHealth@EU"
  "cross-border-identification-2026-2099|32026R2099|http://data.europa.eu/eli/reg_impl/2026/2099/oj||29000|Commission Implementing Regulation (EU) 2026/2099 — interoperable, cross-border identification and authentication mechanism for natural persons, health professionals and healthcare providers"
  "ehr-exchange-format-2019-243|32019H0243|http://data.europa.eu/eli/reco/2019/243/oj||55000|Commission Recommendation (EU) 2019/243 — European Electronic Health Record exchange format"
  "market-surveillance-2019-1020|02019R1020-20260812|http://data.europa.eu/eli/reg/2019/1020/oj|2026-08-12|188000|Regulation (EU) 2019/1020 — market surveillance and compliance of products"
  "accreditation-765-2008|02008R0765-20210716|http://data.europa.eu/eli/reg/2008/765/oj|2021-07-16|460000|Regulation (EC) No 765/2008 — requirements for accreditation"
  "cra-product-categories-2025-2392|32025R2392|http://data.europa.eu/eli/reg_impl/2025/2392/oj||52000|Commission Implementing Regulation (EU) 2025/2392 — technical description of the categories of important and critical products with digital elements"
  "cra-dissemination-delay-2026-881|32026R0881|http://data.europa.eu/eli/reg_del/2026/881/oj||19500|Commission Delegated Regulation (EU) 2026/881 — terms and conditions for applying the cybersecurity-related grounds in relation to delaying the dissemination of notifications"
  "cra-exclusion-vehicles-2025-1535|32025R1535|http://data.europa.eu/eli/reg_del/2025/1535/oj||9000|Commission Delegated Regulation (EU) 2025/1535 — exclusion from the Cyber Resilience Act of certain products with digital elements within the scope of Regulation (EU) No 168/2013"
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
  "cra|corrigendum-2024-12-05.html|32024R2847R%2801%29|application/xhtml+xml|2700|Corrigendum"
  "cra|corrigendum-2025-07-02.html|32024R2847R%2802%29|application/xhtml+xml|2500|Corrigendum"
  "cra|corrigendum-2025-10-17.html|32024R2847R%2804%29|application/xhtml+xml|3000|Corrigendum"
  "nis2|corrigendum-2023-12-22.html|32022L2555R%2804%29|application/xhtml+xml|2100|Corrigendum"
)

# directory | PDF URL | document page | adoption (the record's cell) | byte floor | SHA-256 pin | title
#
# EDPB guidelines, one PDF each. The EDPB has no dated or versioned URI scheme
# a pin could name: a version is a new upload under a new file name, and the
# site can replace the bytes behind an existing name. The pin is therefore the
# digest of the PDF read when the version was set, and a fetch answering other
# bytes stops the run; moving it means reading the new PDF, checking its
# version history page, and changing the digest, title and adoption cell here.
# The byte floor follows the rule of the ACTS table above.
EDPB_DOCS=(
  "edpb-guidelines-01-2025-pseudonymisation|https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf|https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en|16 January 2025 (version 1.0, for public consultation)|300000|db1b9931b3403fab8bab846cb5868df776c415589ad925477117bc6b062bd085|Guidelines 01/2025 on pseudonymisation"
  "edpb-guidelines-02-2023-eprivacy-5-3|https://www.edpb.europa.eu/system/files/documents/2024-10/edpb_guidelines_202302_technical_scope_art_53_eprivacydirective_v2_en_0.pdf|https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en|7 October 2024 (version 2.0, after public consultation)|270000|dbc1d37783e35ae8668925f92a590ae282eec24c98381a5a8f91b5c2048b5b03|Guidelines 2/2023 on Technical Scope of Art. 5(3) of ePrivacy Directive"
)

# The directories named on the command line, each checked against the tables
# so a typo stops the run instead of vendoring nothing.
ONLY=("$@")
for want in "${ONLY[@]+"${ONLY[@]}"}"; do
  known=0
  for entry in "${ACTS[@]}" "${EDPB_DOCS[@]}"; do
    [[ "${entry%%|*}" == "$want" ]] && known=1
  done
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
  nis2) printf '%s' "The cybersecurity directive, transposed into national law, whose security and incident-reporting duties reach the essential and important entities that deploy this software, among them hospitals." ;;
  cra) printf '%s' "The horizontal cybersecurity regulation for products with digital elements, which EHDS Chapter III cross-references for the essential requirements an EHR system inherits." ;;
  mdr) printf '%s' "The medical-device regulation behind the Article 27 interoperability question the EHDS readiness page leaves open for a deployment that claims interoperability with a device." ;;
  eprivacy) printf '%s' "The directive whose Article 5(3) governs storing information in, or gaining access to information stored in, the terminal equipment of a subscriber or user, which the usage report's default is read against (#3580)." ;;
  myhealth-eu-2026-2083) printf '%s' "An EHDS implementing act, adopted under Article 23(4) and (8) of Regulation (EU) 2025/327 and applying from 26 March 2027 (its Article 19); the EHDS readiness and technical-documentation pages state whether the EHDS implementing acts have been adopted, and this is one of the adopted ones (#3607)." ;;
  cross-border-identification-2026-2099) printf '%s' "An EHDS implementing act, adopted under Article 16(2) of Regulation (EU) 2025/327 and applying from 26 March 2027, its Article 3(3) and Article 5(2) from 26 March 2029 (its Article 9); the EHDS readiness and technical-documentation pages state whether the EHDS implementing acts have been adopted, and this is one of the adopted ones (#3607)." ;;
  ehr-exchange-format-2019-243) printf '%s' "The Recommendation that recital 26 of Regulation (EU) 2025/327 says \"provides the foundations\" for the European electronic health record exchange format (#3607)." ;;
  market-surveillance-2019-1020) printf '%s' "The market-surveillance regulation whose Article 3 definitions, 'placing on the market', 'manufacturer' and 'economic operator' among them, Article 2(1)(d) of Regulation (EU) 2025/327 takes over, and which Article 43(1) of that Regulation applies to EHR systems (#3607)." ;;
  accreditation-765-2008) printf '%s' "The regulation whose Article 30 sets the general principles of the CE marking, which Article 41(3) of Regulation (EU) 2025/327 applies to the CE marking of an EHR system and Article 29 of Regulation (EU) 2024/2847 to a product with digital elements (#3607)." ;;
  cra-product-categories-2025-2392) printf '%s' "Adopted under Article 7(4) of Regulation (EU) 2024/2847: its Annex I gives the technical description of the important product categories (classes I and II of Annex III to that Regulation) and its Annex II that of the critical product categories (Annex IV), which a manufacturer reads to see whether its product falls into one (#3607)." ;;
  cra-dissemination-delay-2026-881) printf '%s' "Adopted under Article 14(9) of Regulation (EU) 2024/2847: it sets the conditions under which the CSIRT that first receives a manufacturer's Article 14 notification may delay passing it on under Article 16(2), which SECURITY.md and the post-market procedure cite (#3627)." ;;
  cra-exclusion-vehicles-2025-1535) printf '%s' "Adopted under Article 2(5), second subparagraph, of Regulation (EU) 2024/2847: it excludes from that Regulation the products with digital elements within the scope of Regulation (EU) No 168/2013 (two- or three-wheel vehicles and quadricycles), and is vendored so that a reading of the Regulation's scope covers every delegated act limiting it (#3627)." ;;
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
  cra/corrigendum-2024-12-05.html) printf '%s' "The English corrigendum of OJ L, 2024/90780, 5.12.2024, to the title." ;;
  cra/corrigendum-2025-07-02.html) printf '%s' "The English corrigendum of OJ L, 2025/90555, 2.7.2025, to Article 64(10)." ;;
  cra/corrigendum-2025-10-17.html) printf '%s' "The English corrigendum of OJ L, 2025/90828, 17.10.2025, to Article 67." ;;
  nis2/corrigendum-2023-12-22.html) printf '%s' "The English corrigendum of OJ L, 2023/90206, 22.12.2023, to Article 19(1)." ;;
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

EUR-Lex publishes no consolidation that folds the amendment in. On 2026-10-06
the only consolidated version the Publications Office lists for this act is
`02024R2847-20241120`, the initial one, and EUR-Lex's document metadata, read
2026-10-05, gives 26 March 2027 as the date from which the three changes
apply.

## The corrigenda, and which file to cite for what

The Publications Office records seven corrigenda to this Regulation, CELEX
`32024R2847R(01)` to `32024R2847R(07)` (read 2026-10-06). Three have an
English version, and all three are vendored beside the OJ text:

- `corrigendum-2024-12-05.html`, CELEX `32024R2847R(01)`, OJ L, 2024/90780:
  the title reads "Regulations (EU) No 168/2013 and (EU) 2019/1020" in place
  of "(EU) No 2019/1020".
- `corrigendum-2025-07-02.html`, CELEX `32024R2847R(02)`, OJ L, 2025/90555:
  the introductory wording of Article 64(10) reads "By way of derogation from
  paragraphs 2 to 9" in place of "paragraphs 3 to 9".
- `corrigendum-2025-10-17.html`, CELEX `32024R2847R(04)`, OJ L, 2025/90828:
  the point Article 67 adds to Annex I to Directive (EU) 2020/1828 is
  numbered "72." in place of "69.".

The other four correct other language versions only: `R(03)` French and
Hungarian, `R(05)` Slovak, `R(06)` French, `R(07)` German.

`text.html` is the OJ text of 20 November 2024 and includes none of the three
English corrigenda: its Article 64(10) still reads "paragraphs 3 to 9".
Every other article is cited from `text.html`:
`docs/law/eu/cra/text.html Art. 13(1)`. A citation of Article 64(10) or
Article 67, or of the title, reads `text.html` together with the corrigendum
for it, and cites both.

## Why the OJ text stays pinned

The consolidation `02024R2847-20241120` (its header prints the version
"000.003") folds in all three English corrigenda (marked ►C1 to ►C3 in it)
and no amendment. The rule this script pins by moves an act to a
consolidated CELEX when an amendment has been folded in, and none has: the
EHDS Article 104 changes apply from 26 March 2027 and are not in it. The OJ
text therefore stays the pin, with the corrigenda beside it, and it keeps the
recitals, which the consolidation does not carry. When EUR-Lex publishes a
consolidation that folds in the EHDS amendment, the pin moves to it.
EOF
    ;;
  nis2)
    cat <<'EOF'
## The corrigendum, and which file to cite for what

The Publications Office records nine corrigenda to this Directive, CELEX
`32022L2555R(01)` to `32022L2555R(09)` (read 2026-10-06). One has an English
version, `R(04)`, and it is vendored beside the OJ text:

- `corrigendum-2023-12-22.html`, CELEX `32022L2555R(04)`, OJ L, 2023/90206,
  22.12.2023: Article 19(1), first sentence, reads "The Cooperation Group
  shall, by 17 January 2025, establish" in place of "on 17 January 2025".

The other eight correct other language versions only: `R(01)` Italian and
Dutch, `R(02)` Dutch, `R(03)` Slovenian, `R(05)` Croatian, Maltese, Romanian,
Slovak, Slovenian and Swedish, `R(06)` Estonian, `R(07)` Italian, `R(08)`
French and Hungarian, `R(09)` Estonian and Polish. `R(04)` also has German,
Estonian, Hungarian, Italian and Swedish versions; only the English one is
vendored.

`text.html` is the OJ text of 27 December 2022 and does not include the
corrigendum: its Article 19(1) still reads "on 17 January 2025". Every other
article is cited from `text.html`: `docs/law/eu/nis2/text.html Art. 23(1)`. A
citation of Article 19(1) reads `text.html` together with the corrigendum and
cites both.

## Why the OJ text stays pinned

The Publications Office lists one consolidated version of this Directive,
`02022L2555-20221227` (read 2026-10-06; its header prints the version
"000.004"). It folds in the English corrigendum, marked ►C1 in it, and no
amendment. The rule this script pins by moves an act to a consolidated CELEX
when an amendment has been folded in, and none has, so the OJ text stays the
pin, with the corrigendum beside it, and keeps the recitals the consolidation
does not carry.
EOF
    ;;
  cra-dissemination-delay-2026-881)
    cat <<'EOF'
## Whom it binds, and its corrigendum

Its Articles 3 to 5 are addressed to the CSIRT designated as coordinator that
first receives a notification: they say when that CSIRT may hold a
notification back from the other CSIRTs. A manufacturer has no duty under it.
What a manufacturer reads in it is Article 3(a): the delay is open where "the
manufacturer has informed the CSIRT initially receiving the notification that
an effective risk mitigation measure, such as a security update or user
guidance, is expected to be made available within 72 hours".

The Publications Office records one corrigendum, CELEX `32026R0881R(01)`, in
German only, and one consolidated version, `02026R0881-20260420`, also in
German only (read 2026-10-06). Neither touches the English text, which is
pinned as published in OJ L, 2026/881, 20.4.2026. Cite it as
`docs/law/eu/cra-dissemination-delay-2026-881/text.html Art. 3`.
EOF
    ;;
  cra-exclusion-vehicles-2025-1535)
    cat <<'EOF'
## What it excludes

Its Article 1 excludes the application of Regulation (EU) 2024/2847 for
products with digital elements within the scope of Regulation (EU) No
168/2013, except L1e vehicles designed to pedal. An EHR system is not such a
product, so the exclusion does not reach FerroEHR. Of the acts the
Publications Office records as based on Regulation (EU) 2024/2847 (read
2026-10-06), it is the only one adopted under Article 2(5). It has no
corrigendum and no consolidated version. Cite it as
`docs/law/eu/cra-exclusion-vehicles-2025-1535/text.html Art. 1`.
EOF
    ;;
  ehr-exchange-format-2019-243)
    cat <<'EOF'
## Citing it

A recommendation has no articles. The operative part is a run of numbered
points, (1) to (21), under headings, after "HAS ADOPTED THIS
RECOMMENDATION:"; the points carry no anchor in the XHTML. The recitals carry
`id="rct_1"` to `id="rct_19"`. A citation names the point:
`docs/law/eu/ehr-exchange-format-2019-243/text.html point (11)`.

The Publications Office lists no consolidation and no corrigendum for this
Recommendation (read 2026-10-06).
EOF
    ;;
  market-surveillance-2019-1020)
    cat <<'EOF'
## What the consolidation folds in, and what it does not

`text.html` is the consolidation at 12 August 2026, the latest the
Publications Office lists (read 2026-10-06; the earlier ones are dated
2019-06-25, 2024-02-18 and 2024-05-23). Its header lists three amending acts,
Regulation (EU) 2023/1542 (►M1), Regulation (EU) 2024/1252 (►M2) and
Regulation (EU) 2025/40 (►M3), and the corrigendum to Regulation (EU)
2024/1252 of OJ L, 2024/90589, 1.10.2024 (►C1). Cite the articles from here:
`docs/law/eu/market-surveillance-2019-1020/text.html Art. 3`.

- Article 66 of Regulation (EU) 2024/2847 adds point 72 to Annex I of this
  Regulation, and that Regulation applies from 11 December 2027 (its Article
  71(2)). The consolidation does not carry point 72; read it in
  `docs/law/eu/cra/text.html` Art. 66.
- The consolidation carries no recital. The OJ text is not vendored beside it
  because no page cites a recital of this Regulation yet; when one does, it
  is added through the EXTRAS table.
- The Publications Office records twelve corrigenda to this Regulation,
  CELEX `32019R1020R(01)` to `32019R1020R(12)`, and none has an English
  version (read 2026-10-06).
EOF
    ;;
  accreditation-765-2008)
    cat <<'EOF'
## What the consolidation folds in

`text.html` is the consolidation at 16 July 2021, the latest the Publications
Office lists (read 2026-10-06). It folds in one amending act, Regulation (EU)
2019/1020 (►M1), whose Article 39(1) replaced the title (the words "and
market surveillance relating to the marketing of products" are gone),
deleted Article 1(2) and (3), points 1, 2, 14, 15, 17, 18 and 19 of Article
2, and Chapter III (Articles 15 to 29), and amended Article 32(1). Regulation
(EU) 2019/1020 applies from 16 July 2021 (its Article 44). Market
surveillance is therefore read in
`docs/law/eu/market-surveillance-2019-1020/`, and this text is cited for
accreditation, the definitions that remain and the CE marking:
`docs/law/eu/accreditation-765-2008/text.html Art. 30`.

Article 30 and Annex II (the CE marking, which this file carries as an
embedded image) are unchanged by the amendment. The consolidation carries no
recital, and the OJ text is not vendored beside it. The Publications Office
records five corrigenda to this Regulation, CELEX `32008R0765R(01)` to
`32008R0765R(05)`, and none has an English version (read 2026-10-06).
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

# Why each document is here, written into its PROVENANCE.md above the line
# every EDPB record carries: guidelines bind nobody.
edpb_reason() {
  case "$1" in
  edpb-guidelines-01-2025-pseudonymisation)
    cat <<'EOF'
The supervisory authorities' own reading of what pseudonymisation is under
GDPR Art. 4(5) and what it does to the risk analysis under Art. 32 — the
document the pseudonymisation boundary in this software is designed against.
EOF
    ;;
  edpb-guidelines-02-2023-eprivacy-5-3)
    cat <<'EOF'
The supervisory authorities' reading of which technical operations Article
5(3) of Directive 2002/58/EC reaches; the Directive itself is vendored at
`docs/law/eu/eprivacy/`. The usage report's default is read against it
(#3580): paragraph 18 on terminal equipment that serves the legitimate
interests of legal persons, paragraph 33 on software that proactively calls
an API endpoint.
EOF
    ;;
  *) ;;
  esac
}

# A section only some records need, written from the PDF and its document
# page as read on the date the pin was set.
edpb_note() {
  case "$1" in
  edpb-guidelines-01-2025-pseudonymisation)
    cat <<'EOF'
## Version

Checked on the EDPB site on 2026-10-06: the version for public consultation
adopted on 16 January 2025 is still the current EDPB text of these
guidelines. No later version has been adopted.

- The document page above reads "Closed for feedback", gives the feedback
  period as 17 January to 14 March 2025, and offers two downloads: this PDF
  and "Summary: pseudonymisation, when and how to apply it". The page's
  former address,
  `https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en`,
  redirects to it.
- The EDPB's documents listing, filtered to guidelines, has no entry for
  01/2025. Guidelines finalised after consultation are listed there, as 2/2023
  is.
- Guidelines 02/2026 on anonymisation, adopted for public consultation on 8
  July 2026, are a separate document and are not vendored.

The PDF prints no version number and no version history: its cover reads
"Adopted on 16 January 2025" and every page footer "Adopted - version for
public consultation". "Version 1.0" in this record and in
`docs/law/README.md` names that first and only published version.
EOF
    ;;
  edpb-guidelines-02-2023-eprivacy-5-3)
    cat <<'EOF'
## Version

The version history on page 2 of the PDF lists two versions: 1.0, adopted 14
November 2023 for public consultation, and 2.0, adopted 7 October 2024 after
it. Version 2.0 is vendored; its cover reads "Version 2.0" and "Adopted on 7
October 2024". The document page shows the date 16 October 2024 and the label
"Final version", and links version 1.0 as the first version "drafted before
public consultation". Version 1.0 is not vendored.

The document page also offers version 2.0 in 22 other languages, whose file
paths are dated 2025-02. Only the English PDF is vendored.

## Citing it

The guidelines number their paragraphs from 1 to 63 through the whole
document, footnotes separately. A citation names the paragraph:
`docs/law/eu/edpb-guidelines-02-2023-eprivacy-5-3/guidelines.pdf para. 33`.
EOF
    ;;
  *) ;;
  esac
}

for entry in "${EDPB_DOCS[@]}"; do
  IFS='|' read -r dir url page adopted floor pin title <<<"$entry"
  selected "$dir" || continue
  out="$DEST/$dir"
  echo "==> eu/$dir"

  # Fetched beside the tree and checked before the old directory is touched,
  # so a refused fetch leaves the vendored record as it was.
  staging="$(mktemp -d)"
  bytes="$(fetch "$url" "$staging/guidelines.pdf" "$floor" "" "")"
  if [[ "$(head -c 5 "$staging/guidelines.pdf")" != "%PDF-" ]]; then
    echo "ERROR: $url did not answer with a PDF" >&2
    exit 1
  fi
  digest="$(sha256_of "$staging/guidelines.pdf")"
  if [[ "$digest" != "$pin" ]]; then
    echo "ERROR: $url answered SHA-256 $digest, not the pinned $pin. The EDPB replaced the file; read the new PDF and its version history before moving the pin" >&2
    exit 1
  fi
  rm -rf "$out"
  mkdir -p "$out"
  mv "$staging/guidelines.pdf" "$out/guidelines.pdf"
  rmdir "$staging"

  note_block="$(edpb_note "$dir")"
  [[ -z "$note_block" ]] || note_block=$'\n'"$note_block"$'\n'

  cat >"$out/PROVENANCE.md" <<EOF
# EDPB $title

$(edpb_reason "$dir")
Guidelines are not law: they bind nobody, and this record says so rather than
letting a vendored PDF read like an act.

| | |
|---|---|
| Adopted | $adopted |
| Document page | $page |
| Source | $url |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-eu.sh\` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`guidelines.pdf\` | $bytes | \`$digest\` |

**No text format is published.** The EDPB serves this document as PDF and
offers no HTML, XML or plain-text edition of it, so the PDF is vendored as
published. It is not converted: a converted copy would be this repository's
rendering, and a paragraph number cited against it would resolve to our
pagination rather than the EDPB's.
${note_block}
## Licence

Reuse of EDPB material is authorised for commercial and non-commercial
purposes on the conditions its copyright page states — acknowledge the source,
do not distort the meaning, and the EDPB carries no liability for the reuse.
The page is quoted verbatim in
\`LICENSES/LicenseRef-EDPB-Reuse.txt\` and declared for this directory in
\`REUSE.toml\`. Source acknowledged: European Data Protection Board,
$title, $page.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-eu.sh\` instead.
EOF
  write_sha256sums "$out"
  echo "    guidelines.pdf: $bytes bytes"
done

echo "Done. Vendored into $DEST"
