#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendors the Dutch acts the compliance pages cite into docs/law/nl/, one
# directory per act, in the format wetten.overheid.nl serves, plus the records
# for the three NEN standards that may not be vendored at all.
#
# WHY THE TEXTS ARE IN THE TREE. Same reason as the EU layer
# (scripts/vendor/law-eu.sh): a compliance claim checked against a page fetched
# at review time is checked against whatever that page says on the day. A
# consolidation vendored at a named date is a fixed referent, and SHA256SUMS
# proves the bytes are the ones the script fetched.
#
# FORMAT AND PIN. wetten.overheid.nl serves a consolidated act as HTML at
# wetten.overheid.nl/<BWB-id>/<yyyy-mm-dd>, and that HTML is vendored verbatim
# as text.html. The date is pinned in the table below rather than resolved at
# run time, so a re-run reproduces the same consolidation instead of silently
# following the site to a newer one; each pin is the consolidation the
# undated URL redirected to when the pin was set.
#
# ONE THING THAT IS NOT REPRODUCIBLE, and it is the publisher's doing: the
# served page carries a "Geraadpleegd op <date>" line, so re-fetching on a
# later day changes the bytes even when the act has not changed a word. The
# SHA-256 recorded here is of the fetch this record names, not a property of
# the consolidation.
#
# LICENCE. Auteurswet Art. 11: no copyright subsists in laws, decrees and
# ordinances issued by the public authority. The article is quoted verbatim in
# LICENSES/LicenseRef-Auteurswet-Art11-Public-Domain.txt and declared for this
# subtree in REUSE.toml.
#
# THE NEN STANDARDS ARE NOT HERE, AND THAT IS DELIBERATE. NEN 7510-1:2024,
# NEN 7510-2:2024, NEN 7512:2022 and NEN 7513:2018 are sold under copyright by
# NEN. They are not fetched, not vendored, and not quoted. Each gets a
# PROVENANCE.md that says so, names the edition and the shop URL, and lists
# what the compliance pages cite, so a reviewer holding a licensed copy can
# follow every citation.
#
# Idempotent: each act directory is wiped and re-fetched, each NEN record is
# rewritten from this script. With no argument every directory is written; with
# arguments, only the named ones. Because of the consultation stamp above, a
# full re-run changes the digest of every act, so adding one act names it:
#
#   scripts/vendor/law-nl.sh                       # every act and record
#   scripts/vendor/law-nl.sh telecommunicatiewet   # that act only
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$REPO_ROOT/docs/law/nl"

# The publisher sees a named client rather than a default curl string, so a
# fetch from this repository is attributable at the far end.
UA='ferroehr-vendor/1.0 (+https://github.com/FerroHEALTH/FerroEHR)'
FETCHED="$(date -u +%Y-%m-%d)"

# directory | BWB id | consolidation date | byte floor | title
#
# The byte floor is roughly two thirds of the size measured when the pin was
# set — enough to tell an act from the login, consent or error page a publisher
# answers with when something goes wrong. The logging decree is genuinely one
# sentence long, which is why its floor is the small one.
ACTS=(
  "uavg|BWBR0040940|2026-09-01|300000|Uitvoeringswet Algemene verordening gegevensbescherming (UAVG)"
  "wabvpz|BWBR0023864|2025-07-05|150000|Wet aanvullende bepalingen verwerking persoonsgegevens in de zorg (Wabvpz)"
  "bw7-geneeskundige-behandelingsovereenkomst|BWBR0005290|2026-07-01|4000000|Burgerlijk Wetboek Boek 7 (Bijzondere overeenkomsten)"
  "besluit-bewaartermijn-logging|BWBR0042391|2019-09-01|20000|Besluit vaststelling bewaartermijn logging"
  "begz|BWBR0040238|2020-10-01|50000|Besluit elektronische gegevensverwerking door zorgaanbieders (Begz)"
  "telecommunicatiewet|BWBR0009950|2026-08-15|2000000|Telecommunicatiewet"
)

NEN_RECORDS=(nen-7510 nen-7512 nen-7513)

# The directories named on the command line, each checked against the tables
# so a typo stops the run instead of writing nothing.
ONLY=("$@")
for want in "${ONLY[@]+"${ONLY[@]}"}"; do
  known=0
  for entry in "${ACTS[@]}"; do
    [[ "${entry%%|*}" == "$want" ]] && known=1
  done
  for nen in "${NEN_RECORDS[@]}"; do
    [[ "$nen" == "$want" ]] && known=1
  done
  if [[ "$known" -eq 0 ]]; then
    echo "ERROR: '$want' is not a directory this script writes" >&2
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

# Why each act is here, and which part of it the pages cite.
act_reason() {
  case "$1" in
  uavg) printf '%s' "The Dutch implementation of the GDPR: the national derogations, the Art. 9 health-data grounds and the BSN rules a deployment in the Netherlands processes under." ;;
  wabvpz) printf '%s' "The act behind the electronic exchange of health records in the Netherlands, including the patient's right to an overview of who accessed their record — the obligation the access log serves." ;;
  bw7-geneeskundige-behandelingsovereenkomst) printf '%s' "The medical treatment contract (Wgbo), **Art. 7:446 to 7:468** — the retention obligation, the right of access and the duty of confidentiality the clinical record is kept under. The whole book is vendored, because the publisher serves it as one document and cutting the cited articles out of it would be an edit." ;;
  besluit-bewaartermijn-logging) printf '%s' "The decree that fixes the five-year minimum retention for the access log, and the source the audit retention floor for the NL jurisdiction is set from." ;;
  begz) printf '%s' "The decree on electronic processing by care providers: Art. 5 requires the logging and delegates its retention period, which is what the decree above then fixes." ;;
  telecommunicatiewet) printf '%s' "The Dutch electronic-communications act, vendored for **Art. 11.7a**, the rule on storing information in, or gaining access to information in, a user's terminal equipment (\"randapparatuur\"), which the usage report's default is read against (#3580). The whole act is vendored, because the publisher serves it as one document and cutting the article out of it would be an edit." ;;
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
    echo "ERROR: $url returned $bytes bytes, under the $floor-byte floor — that is a login, consent or error page, not an act" >&2
    exit 1
  fi
  if [[ -n "$needle" ]] && ! grep -qi -- "$needle" "$out"; then
    echo "ERROR: $url returned $bytes bytes that never say '$needle' — that is not the act text" >&2
    exit 1
  fi
  printf '%s' "$bytes"
}

mkdir -p "$DEST"

for entry in "${ACTS[@]}"; do
  IFS='|' read -r dir bwb consolidated floor title <<<"$entry"
  selected "$dir" || continue
  url="https://wetten.overheid.nl/$bwb/$consolidated"
  out="$DEST/$dir"
  echo "==> nl/$dir ($bwb @ $consolidated)"
  rm -rf "$out"
  mkdir -p "$out"

  bytes="$(fetch "$url" "$out/text.html" "$floor" "Artikel")"
  digest="$(sha256_of "$out/text.html")"

  cat >"$out/PROVENANCE.md" <<EOF
# $title

$(act_reason "$dir")

| | |
|---|---|
| BWB identifier | \`$bwb\` |
| Consolidation vendored | $consolidated (geldend van) |
| Source | $url |
| Undated permalink | https://wetten.overheid.nl/$bwb |
| Fetched | $FETCHED (UTC) |
| Vendored by | \`scripts/vendor/law-nl.sh\` |

## Files

| file | bytes | SHA-256 |
|---|---|---|
| \`text.html\` | $bytes | \`$digest\` |

The HTML wetten.overheid.nl serves for this consolidation, byte for byte.
Nothing was converted or extracted: an extraction is an edit, and an edited act
is no longer the publisher's text.

The served page stamps its own consultation date into the markup, so re-running
the script on a later day changes these bytes even if the act has not changed a
word. The digest above is of the fetch this record names.

## Licence

No copyright subsists in this text. Auteurswet Art. 11: *"Er bestaat geen
auteursrecht op wetten, besluiten en verordeningen, door de openbare macht
uitgevaardigd, noch op rechterlijke uitspraken en administratieve
beslissingen."* The article is quoted in
\`LICENSES/LicenseRef-Auteurswet-Art11-Public-Domain.txt\` and declared for
this subtree in \`REUSE.toml\`.

Do not hand-edit anything in this directory. Re-run
\`scripts/vendor/law-nl.sh\` instead — a hand edit makes the SHA256SUMS line a
lie, which is the one thing a vendored legal text may never be.
EOF

  write_sha256sums "$out"
  echo "    text.html: $bytes bytes"
done

# ── The NEN standards: a record, and nothing else ──────────────────────────
# Sold under copyright by NEN. Nothing is fetched and nothing is quoted; each
# directory carries the record a reviewer with a licensed copy needs to follow
# the citations, and no SHA256SUMS, because there is nothing to hash.
nen_record() {
  local dir="$1"
  echo "==> nl/$dir (record only — the standard is copyright NEN)"
  rm -rf "${DEST:?}/$dir"
  mkdir -p "$DEST/$dir"
}

if selected nen-7510; then
  nen_record nen-7510
  cat >"$DEST/nen-7510/PROVENANCE.md" <<EOF
# NEN 7510-1:2024 and NEN 7510-2:2024 — not vendored

**Nothing but this record is in this directory, and nothing else may be.**
NEN 7510 is sold under copyright by NEN (Koninklijk Nederlands
Normalisatie-instituut). It is not free to redistribute, so the standard is not
fetched, not vendored, not quoted and not paraphrased clause by clause here.

| | |
|---|---|
| NEN 7510-1:2024 | Medische informatica — Informatiebeveiliging in de zorg — Deel 1: Managementsysteem |
| NEN 7510-2:2024 | Medische informatica — Informatiebeveiliging in de zorg — Deel 2: Beheersmaatregelen |
| Publisher | NEN, https://www.nen.nl |
| Part 1 | https://www.nen.nl/nen-7510-1-2024-nl-331311 |
| Part 2 | https://www.nen.nl/nen-7510-2-2024-nl-331314 |
| Recorded | $FETCHED (UTC) by \`scripts/vendor/law-nl.sh\` |

## What the compliance pages cite

The citations are to a PART and a control THEME rather than to a numbered
clause: the pages state which half of the standard an obligation sits in and
which control family it belongs to, never a clause number. A reviewer with a
licensed copy follows them by control family.

| Cited as | Where | What the page claims against it |
|---|---|---|
| NEN 7510-1, the information security management system | \`website/book/src/compliance/shared-responsibility.md\`, \`website/book/src/compliance/index.md\` | The software supplies technical controls an ISMS can point at; running the ISMS, and holding any certificate, is the deployment's |
| NEN 7510-2, the control set | \`website/book/src/compliance/shared-responsibility.md\`, \`website/book/src/compliance/index.md\` | Access control, audit logging, cryptography in transit and for version signatures, supply-chain verification |
| NEN 7510-2, cryptographic controls | \`website/book/src/security/dpia.md\` | National identifiers sealed and looked up by keyed digest, resolved under audit |

## Why the citations still work without the text

The claims above are claims about THIS software, checkable in this repository:
the control is in the code, its behaviour is in the tests, and the page names
which. What the standard requires is checkable only against the standard, by
someone who holds it. That split is the reason this record exists instead of a
copy.

Do not add files to this directory. Re-run \`scripts/vendor/law-nl.sh\` to
rewrite this record.
EOF
fi

if selected nen-7512; then
  nen_record nen-7512
  cat >"$DEST/nen-7512/PROVENANCE.md" <<EOF
# NEN 7512:2022 — not vendored

**Nothing but this record is in this directory, and nothing else may be.**
NEN 7512 is sold under copyright by NEN (Koninklijk Nederlands
Normalisatie-instituut). It is not free to redistribute, so the standard is not
fetched, not vendored, not quoted and not paraphrased clause by clause here.

| | |
|---|---|
| NEN 7512:2022 | Medische informatica — Informatiebeveiliging in de zorg — Vertrouwensbasis voor gegevensuitwisseling |
| Publisher | NEN, https://www.nen.nl |
| Shop | https://www.nen.nl/nen-7512-2022-nl-297137 |
| Recorded | $FETCHED (UTC) by \`scripts/vendor/law-nl.sh\` |

## What the compliance pages cite

No numbered clause is cited. The standard is cited whole, as the trust basis
two exchanging parties agree on.

| Cited as | Where | What the page claims against it |
|---|---|---|
| NEN 7512, the trust basis for data exchange | \`website/book/src/compliance/shared-responsibility.md\`, \`website/book/src/compliance/index.md\` | Mutually authenticated TLS (IHE ITI-19), OAuth2 and OIDC against an enterprise identity provider, SMART App Launch, and signed verifiable releases; agreeing the trust basis with each counterparty and operating the certificate estate stays the deployment's |

## Why the citations still work without the text

The claims above are claims about THIS software, checkable in this repository.
What the standard requires is checkable only against the standard, by someone
who holds it.

Do not add files to this directory. Re-run \`scripts/vendor/law-nl.sh\` to
rewrite this record.
EOF
fi

if selected nen-7513; then
  nen_record nen-7513
  cat >"$DEST/nen-7513/PROVENANCE.md" <<EOF
# NEN 7513:2018 — not vendored

**Nothing but this record is in this directory, and nothing else may be.**
NEN 7513 is sold under copyright by NEN (Koninklijk Nederlands
Normalisatie-instituut). It is not free to redistribute, so the standard is not
fetched, not vendored, not quoted and not paraphrased clause by clause here.

| | |
|---|---|
| NEN 7513:2018 | Medische informatica — Logging — Vastleggen van acties op elektronische patiëntdossiers |
| Publisher | NEN, https://www.nen.nl |
| Shop | https://www.nen.nl/nen-7513-2018-nl-245399 |
| Recorded | $FETCHED (UTC) by \`scripts/vendor/law-nl.sh\` |

## What the compliance pages cite

This is the one standard of the three whose citations reach a numbered clause.

| Cited as | Where | What the page claims against it |
|---|---|---|
| §5, the content of a logged event | \`website/book/src/audit.md\` (the field-by-field table) | Every item the clause lists is mapped to the field of the access record that carries it, including the two a deployment must supply itself (\`purpose\`, \`legal_basis\`) |
| Actor role | \`website/book/src/compliance/control-matrix.md\`, \`website/book/src/security/dpia.md\` | The access record carries the roles the caller held at access time, as FHIR \`agent.role\` and as a DICOM \`RoleIDCode\` |
| Event content | \`website/book/src/compliance/control-matrix.md\`, \`website/book/src/security/dpia.md\`, \`website/book/src/security/records-of-processing.md\` | Per-domain access logging for reads and queries, aligned with this standard and EHDS Annex II 3.2 |
| Retention | \`website/book/src/audit.md\`, \`website/book/src/compliance/control-matrix.md\`, \`website/book/src/security/dpia.md\` | The retention floor for the NL jurisdiction, which the Besluit vaststelling bewaartermijn logging binds to this standard |

The retention chain is followable without holding the standard, because the
legal end of it is vendored: \`docs/law/nl/begz/\` (Art. 5 delegates the
period) and \`docs/law/nl/besluit-bewaartermijn-logging/\` (the period is five
years from the moment the log line is written).

## Why the citations still work without the text

The claims above are claims about THIS software, checkable in this repository:
the recorded fields are in the schema and in the tests, and the page names
which field answers which item. What the standard requires of the item list is
checkable only against the standard, by someone who holds it.

Do not add files to this directory. Re-run \`scripts/vendor/law-nl.sh\` to
rewrite this record.
EOF
fi

echo "Done. Vendored into $DEST"
