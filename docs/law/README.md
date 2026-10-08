# The regulation corpus

The legal texts the compliance documentation cites, vendored verbatim at a
named consolidation so a citation resolves to bytes this repository carries
rather than to whatever the publisher's page says on the day it is read.

Four scripts, one per jurisdiction, produce the whole tree and nothing here is
written by hand:

| script | destination |
|---|---|
| `scripts/vendor/law-eu.sh` | `docs/law/eu/` |
| `scripts/vendor/law-nl.sh` | `docs/law/nl/` |
| `scripts/vendor/law-de.sh` | `docs/law/de/` |
| `scripts/vendor/law-ch.sh` | `docs/law/ch/` |

Every act directory carries the publisher's own file, a `PROVENANCE.md` naming
the identifier, the consolidation, the source URL, the fetch date and the
licence, and a `SHA256SUMS` with one line per vendored file.
`scripts/checks/law-corpus.sh` verifies the digests and refuses a directory
whose record is missing or names a file that is not there.

**Never hand-edit anything under this tree.** A hand edit makes the recorded
digest a lie, which is the one thing a vendored legal text may never be. To
refresh or re-pin, change the script and re-run it.

**This is reference material, not a build input.** Nothing compiles against it
and no test reads the act text; it is what a reviewer checks a compliance claim
against. The openEHR specification oracle is a separate tree,
`docs/specs/openehr/`.

## European Union

The act text is fetched from the Publications Office CELEX content-negotiation
URI, which answers the same document byte for byte on every request; the
eur-lex.europa.eu page linked below renders that document for a human reader
but wraps it in per-request site chrome, so it is not the fetch source. Each
record names both, and the exact manifestation the CELEX resolved to.

| Act | Identifier | Consolidation vendored | Read it at | Licence | Cited by |
|---|---|---|---|---|---|
| [Regulation (EU) 2016/679, General Data Protection Regulation](eu/gdpr/) | CELEX `02016R0679-20160504`, ELI `reg/2016/679`; beside it the OJ text `32016R0679` (`oj.html`, the recitals) and the 2018 corrigendum `32016R0679R(02)` | 2016-05-04 (articles); the OJ text of 4.5.2016 for the recitals | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02016R0679-20160504) | `LicenseRef-EUR-Lex-Reuse AND CC-BY-4.0` (the consolidation); `LicenseRef-EUR-Lex-Reuse` (the OJ texts) | `compliance/index.md`, `compliance/control-matrix.md`, `compliance/shared-responsibility.md`, `security.md`, `security/dpia.md`, `security/records-of-processing.md`, `audit.md`, `operations.md`, `concepts/storage.md`, `installation/config-privacy.md`, `installation/config-auth.md`, `installation/config-audit.md`, `installation/configuration.md`, `contributing.md` |
| [Regulation (EU) 2025/327, European Health Data Space](eu/ehds/) | CELEX `32025R0327`, ELI `reg/2025/327` | none, the OJ text | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32025R0327) | `LicenseRef-EUR-Lex-Reuse` | `compliance/ehds-readiness.md`, `compliance/technical-documentation.md`, `compliance/index.md`, `compliance/control-matrix.md`, `compliance/shared-responsibility.md`, `audit.md`, `security/dpia.md`, `security/records-of-processing.md`, `comparison.md`, `why-ferroehr.md`, `operations.md`, `installation/config-audit.md`, `installation/config-auth.md`, `installation/configuration.md` |
| [Directive (EU) 2022/2555, NIS2](eu/nis2/) | CELEX `32022L2555`, ELI `dir/2022/2555`; beside it the English corrigendum `32022L2555R(04)` | none, the OJ text, which does not carry the corrigendum (the initial consolidation folds it in and no amendment, so the pin stays at the OJ text); Art. 19(1) is read with it | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32022L2555) | `LicenseRef-EUR-Lex-Reuse` | `compliance/post-market.md` (EHDS Art. 44(7) is without prejudice to it); vendored for the cybersecurity obligations a deployment answers to |
| [Regulation (EU) 2024/2847, Cyber Resilience Act](eu/cra/) | CELEX `32024R2847`, ELI `reg/2024/2847`; beside it the three English corrigenda `32024R2847R(01)`, `R(02)` and `R(04)` | none, the OJ text, which carries none of the corrigenda (the initial consolidation folds them in and no amendment, so the pin stays at the OJ text) | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32024R2847) | `LicenseRef-EUR-Lex-Reuse` | `compliance/ehds-readiness.md`, `compliance/technical-documentation.md` (as the regulation EHDS amends and cross-references); EHDS Art. 104 amends Art. 13(4), 31(3) and inserts 32(5a) from 26 March 2027, which this text does not carry, and Art. 64(10) and 67 are read with their corrigenda (its record says which file to cite) |
| [Regulation (EU) 2017/745, Medical Device Regulation](eu/mdr/) | CELEX `02017R0745-20260719`, ELI `reg/2017/745` | 2026-07-19 | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02017R0745-20260719) | `LicenseRef-EUR-Lex-Reuse AND CC-BY-4.0` | `compliance/ehds-readiness.md` (the Article 27 interoperability question, left open) |
| [Directive 2002/58/EC, ePrivacy Directive](eu/eprivacy/) | CELEX `02002L0058-20091219`, ELI `dir/2002/58`; beside it the OJ texts of `32002L0058` and of the amending Directive `32009L0136`, for the recitals | 2009-12-19, Art. 5(3) as amended by Directive 2009/136/EC | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02002L0058-20091219) | `LicenseRef-EUR-Lex-Reuse AND CC-BY-4.0` (the consolidation); `LicenseRef-EUR-Lex-Reuse` (the OJ texts) | no page yet; vendored for the usage report's default (#3580) |
| [Commission Implementing Regulation (EU) 2026/2083, MyHealth@EU](eu/myhealth-eu-2026-2083/) | CELEX `32026R2083`, ELI `reg_impl/2026/2083` | none, the OJ text (OJ L, 2026/2083, 21.9.2026); applies from 26 March 2027 | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32026R2083) | `LicenseRef-EUR-Lex-Reuse` | no page yet; vendored as an adopted EHDS implementing act (#3607) |
| [Commission Implementing Regulation (EU) 2026/2099, cross-border identification and authentication](eu/cross-border-identification-2026-2099/) | CELEX `32026R2099`, ELI `reg_impl/2026/2099` | none, the OJ text (OJ L, 2026/2099, 22.9.2026); applies from 26 March 2027, Art. 3(3) and 5(2) from 26 March 2029 | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32026R2099) | `LicenseRef-EUR-Lex-Reuse` | no page yet; vendored as an adopted EHDS implementing act (#3607) |
| [Commission Recommendation (EU) 2019/243, European Electronic Health Record exchange format](eu/ehr-exchange-format-2019-243/) | CELEX `32019H0243`, ELI `reco/2019/243` | none, the OJ text (OJ L 39, 11.2.2019, p. 18) | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32019H0243) | `LicenseRef-EUR-Lex-Reuse` | no page yet; EHDS recital 26 names it as the foundation of the exchange format (#3607) |
| [Regulation (EU) 2019/1020, market surveillance and compliance of products](eu/market-surveillance-2019-1020/) | CELEX `02019R1020-20260812`, ELI `reg/2019/1020` | 2026-08-12, folding in Regulations (EU) 2023/1542, 2024/1252 and 2025/40; the CRA's Art. 66 addition, applying from 11 December 2027, is not in it | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02019R1020-20260812) | `LicenseRef-EUR-Lex-Reuse AND CC-BY-4.0` | no page yet; EHDS Art. 2(1)(d) takes its Art. 3 definitions and Art. 43(1) applies it to EHR systems (#3607) |
| [Regulation (EC) No 765/2008, requirements for accreditation](eu/accreditation-765-2008/) | CELEX `02008R0765-20210716`, ELI `reg/2008/765` | 2021-07-16, folding in Regulation (EU) 2019/1020 Art. 39 | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02008R0765-20210716) | `LicenseRef-EUR-Lex-Reuse AND CC-BY-4.0` | no page yet; its Art. 30 (the CE marking) is applied by EHDS Art. 41(3) and CRA Art. 29 (#3607) |
| [Commission Implementing Regulation (EU) 2025/2392, important and critical products with digital elements](eu/cra-product-categories-2025-2392/) | CELEX `32025R2392`, ELI `reg_impl/2025/2392` | none, the OJ text (OJ L, 2025/2392, 1.12.2025) | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32025R2392) | `LicenseRef-EUR-Lex-Reuse` | no page yet; the technical description of the CRA Annex III and IV categories (#3607) |
| [Commission Delegated Regulation (EU) 2026/881, delaying the dissemination of CRA notifications](eu/cra-dissemination-delay-2026-881/) | CELEX `32026R0881`, ELI `reg_del/2026/881` | none, the OJ text (OJ L, 2026/881, 20.4.2026); its one corrigendum and its consolidation exist in German only | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32026R0881) | `LicenseRef-EUR-Lex-Reuse` | `compliance/post-market.md`; adopted under CRA Art. 14(9), read with Art. 16(2), which `SECURITY.md` cites (#3627) |
| [Commission Delegated Regulation (EU) 2025/1535, exclusion of products within Regulation (EU) No 168/2013](eu/cra-exclusion-vehicles-2025-1535/) | CELEX `32025R1535`, ELI `reg_del/2025/1535` | none, the OJ text (OJ L, 2025/1535, 29.10.2025) | [EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32025R1535) | `LicenseRef-EUR-Lex-Reuse` | no page yet; adopted under CRA Art. 2(5), so a reading of the CRA's scope covers it; it does not reach an EHR system (#3627) |
| [EDPB Guidelines 01/2025 on pseudonymisation](eu/edpb-guidelines-01-2025-pseudonymisation/) | EDPB Guidelines 01/2025, version 1.0 | adopted 2025-01-16 | [EDPB](https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en) | `LicenseRef-EDPB-Reuse` | `compliance/index.md`, `compliance/control-matrix.md`, `security.md`, `security/dpia.md`, `operations.md` |
| [EDPB Guidelines 2/2023 on Technical Scope of Art. 5(3) of ePrivacy Directive](eu/edpb-guidelines-02-2023-eprivacy-5-3/) | EDPB Guidelines 2/2023, version 2.0 | adopted 2024-10-07, after public consultation (version 1.0 of 2023-11-14 not vendored) | [EDPB](https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en) | `LicenseRef-EDPB-Reuse` | no page yet; vendored for the usage report's default (#3580), paragraphs 18 and 33 |

### Adopted acts recorded and not vendored

These acts are adopted under the EHDS and checked against it, and none of them
puts a duty on the manufacturer of an EHR system or on the EHR system itself,
so no page cites them and they are not vendored. Each is recorded here so that
its absence is a reading, not an oversight (#3627). If a page comes to cite
one, it is added to `scripts/vendor/law-eu.sh` first.

| Act | Identifier | Adopted under | Why it is not vendored |
|---|---|---|---|
| Commission Implementing Regulation (EU) 2026/771, the measures for the establishment and operation of the European Health Data Space Board | CELEX `32026R0771`, ELI `reg_impl/2026/771` | EHDS Art. 92(11) | Its Article 1 "lays down the necessary rules for the establishment, the management and the functioning" of the EHDS Board, the body of Member State representatives EHDS Art. 92(1) establishes. Its articles address the Board, the Member States and the Commission; none addresses a manufacturer or an EHR system. |
| Commission Implementing Regulation (EU) 2026/2098, the minimum metadata elements health data holders provide for dataset descriptions for secondary use | CELEX `32026R2098`, ELI `reg_impl/2026/2098` | EHDS Art. 77(4) | It concerns secondary use under Chapter IV: its Article 1(1) lays down the metadata elements "that health data holders are required to provide to health data access bodies pursuant to Article 60(3)", and its Article 4 applies it from 26 March 2029. A deploying organisation that is a health data holder reads it for its own duty; it sets no requirement for an EHR system. |

Guidelines are not law. The EDPB documents are here because the
pseudonymisation boundary is designed against 01/2025 and the usage report's
default is read against 2/2023, and each record says plainly that it binds
nobody. The EDPB publishes its guidelines as PDF only, so each is vendored as
`guidelines.pdf`, pinned by its SHA-256 in the vendor script.

## The Netherlands

| Act | Identifier | Consolidation vendored | Read it at | Licence | Cited by |
|---|---|---|---|---|---|
| [Uitvoeringswet AVG (UAVG)](nl/uavg/) | BWB `BWBR0040940` | 2026-09-01 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0040940/2026-09-01) | `LicenseRef-Auteurswet-Art11-Public-Domain` | `compliance/index.md`, `compliance/control-matrix.md`, `compliance/shared-responsibility.md` |
| [Wabvpz](nl/wabvpz/) | BWB `BWBR0023864` | 2025-07-05 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0023864/2025-07-05) | `LicenseRef-Auteurswet-Art11-Public-Domain` | `compliance/index.md`, `compliance/control-matrix.md`, `compliance/shared-responsibility.md`, `audit.md` |
| [Burgerlijk Wetboek Boek 7, geneeskundige behandelingsovereenkomst (Wgbo)](nl/bw7-geneeskundige-behandelingsovereenkomst/) | BWB `BWBR0005290`, Art. 7:446-468 | 2026-07-01 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0005290/2026-07-01) | `LicenseRef-Auteurswet-Art11-Public-Domain` | `compliance/shared-responsibility.md` |
| [Besluit vaststelling bewaartermijn logging](nl/besluit-bewaartermijn-logging/) | BWB `BWBR0042391` | 2019-09-01 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0042391/2019-09-01) | `LicenseRef-Auteurswet-Art11-Public-Domain` | `audit.md`, `security/dpia.md`, `security/go-live-checklist.md` |
| [Besluit elektronische gegevensverwerking door zorgaanbieders (Begz)](nl/begz/) | BWB `BWBR0040238` | 2020-10-01 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0040238/2020-10-01) | `LicenseRef-Auteurswet-Art11-Public-Domain` | `audit.md` |
| [Telecommunicatiewet](nl/telecommunicatiewet/), Art. 11.7a | BWB `BWBR0009950` | 2026-08-15 | [wetten.overheid.nl](https://wetten.overheid.nl/BWBR0009950/2026-08-15) | `LicenseRef-Auteurswet-Art11-Public-Domain` | no page yet; vendored for the usage report's default (#3580) |

The whole of Boek 7 and the whole Telecommunicatiewet are vendored rather than
the cited articles alone: the publisher serves each as one document, and
cutting Art. 446-468 or Art. 11.7a out of it would be an edit.

## Germany

Federal law only. The sixteen Landeskrankenhausgesetze and the state data
protection acts a hospital also answers to are out of scope here; a
deployment reads its own state's law beside this layer.

The consolidated acts come from gesetze-im-internet.de as the XML document
the publisher packs into each act's `xml.zip`, under the publisher's own file
name. The site has no dated URL, so the pin is the publisher's `builddate`
stamp on the document, and the vendor script refuses a fetch whose stamp
differs from the pin. The publisher describes its consolidations as
non-official texts; the authentic text is the Bundesgesetzblatt, which is why
the Digital-Gesetz, an amending act with no consolidation of its own, is
vendored as its BGBl. issue from recht.bund.de.

| Act | Identifier | Consolidation vendored | Read it at | Licence | Cited by |
|---|---|---|---|---|---|
| [Bundesdatenschutzgesetz (BDSG)](de/bdsg/) | `bdsg_2018`, document `BJNR209710017` | builddate 2026-07-13; last amendment folded in: Art. 3 G v. 12.5.2026 I Nr. 139 | [gesetze-im-internet.de](https://www.gesetze-im-internet.de/bdsg_2018/) | `LicenseRef-UrhG-Para5-Amtliche-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Sozialgesetzbuch Fünftes Buch (SGB V)](de/sgb-v/), §§ 341–355 and 360–363 | `sgb_5`, document `BJNR024820988` | builddate 2026-08-10; last amendment folded in: Art. 1 G v. 26.6.2026 I Nr. 195 | [gesetze-im-internet.de](https://www.gesetze-im-internet.de/sgb_5/) | `LicenseRef-UrhG-Para5-Amtliche-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Gesundheitsdatennutzungsgesetz (GDNG)](de/gdng/) | `gdng`, document `BJNR0660B0024` | builddate 2024-04-25; unamended since promulgation (BGBl. 2024 I Nr. 102) | [gesetze-im-internet.de](https://www.gesetze-im-internet.de/gdng/) | `LicenseRef-UrhG-Para5-Amtliche-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Strafgesetzbuch (StGB)](de/stgb/), § 203 | `stgb`, document `BJNR001270871` | builddate 2026-07-02; last amendment folded in: Art. 1 G v. 20.3.2026 I Nr. 95 | [gesetze-im-internet.de](https://www.gesetze-im-internet.de/stgb/) | `LicenseRef-UrhG-Para5-Amtliche-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Digital-Gesetz (DigiG)](de/digig/) | BGBl. 2024 I Nr. 101, ELI `bgbl-1/2024/101` | none, the promulgated issue (an amending act; its substance is the SGB V text above) | [recht.bund.de](https://www.recht.bund.de/bgbl/1/2024/101/VO.html) | `LicenseRef-UrhG-Para5-Amtliche-Werke` | `compliance/index.md` |

The whole of SGB V and the whole of the StGB are vendored rather than the
cited sections alone, for the same reason as Boek 7 above: the publisher
serves each act as one document, and cutting the sections out would be an
edit.

## Switzerland

Swiss federal law from Fedlex, the Federal Chancellery's Classified
Compilation. The text is fetched from the Fedlex filestore, which serves one
HTML document per act, consolidation and language and answers the same bytes
on every request; the www.fedlex.admin.ch page is a single-page application
that answers an identical shell for every act, so it is linked for reading but
is not the fetch source. German, French and Italian are the authentic
languages; German is vendored as the reference. Fedlex's English translations
are vendored beside it only where one exists at the same consolidation, each
carrying the publisher's own "has no legal force" disclaimer, which the
vendor script asserts is present.

| Act | Identifier | Consolidation vendored | Read it at | Licence | Cited by |
|---|---|---|---|---|---|
| [Bundesgesetz über den Datenschutz (DSG / FADP)](ch/fadp/) | SR `235.1`, ELI `cc/2022/491` | 2025-07-07 (German + English) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/2022/491/20250707/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Verordnung über den Datenschutz (DSV / DPO)](ch/dpo/) | SR `235.11`, ELI `cc/2022/568` | 2025-12-01 (German + English) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/2022/568/20251201/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Bundesgesetz über das elektronische Patientendossier (EPDG)](ch/epdg/) | SR `816.1`, ELI `cc/2017/203` | 2024-10-01 (German; English exists only at 2020-04-15, not vendored) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/2017/203/20241001/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Verordnung über das elektronische Patientendossier (EPDV)](ch/epdv/) | SR `816.11`, ELI `cc/2017/204` | 2024-10-01 (German; English exists only at 2019-04-01, not vendored) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/2017/204/20241001/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `compliance/index.md`, `compliance/shared-responsibility.md` |
| [Verordnung des EDI über das elektronische Patientendossier (EPDV-EDI)](ch/epdv-edi/) | SR `816.111`, ELI `cc/2017/205` | 2026-06-01 (German; no English published) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/2017/205/20260601/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `compliance/index.md` |
| [Verordnung über die Alters- und Hinterlassenenversicherung (AHVV)](ch/ahvv/), Art. 133 | SR `831.101`, ELI `cc/63/1185_1183_1185` | 2026-01-01 (German; no English published) | [Fedlex](https://www.fedlex.admin.ch/eli/cc/63/1185_1183_1185/20260101/de) | `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | `installation/config-privacy.md` (the `ch-ahvn13` identifier rule) |

## The standards that cannot be vendored

NEN sells its standards under copyright, so no text of one is in this tree.
Each directory holds a record naming the edition, the shop URL and the clauses
the compliance pages cite, so a reviewer holding a licensed copy can follow
every citation.

| Standard | Record | What the pages cite |
|---|---|---|
| NEN 7510-1:2024 and NEN 7510-2:2024, information security in healthcare | [nl/nen-7510/](nl/nen-7510/) | The part (-1 the management system, -2 the controls) and the control family; no numbered clause |
| NEN 7512:2022, the trust basis for data exchange | [nl/nen-7512/](nl/nen-7512/) | The standard whole; no numbered clause |
| NEN 7513:2018, logging actions on electronic patient records | [nl/nen-7513/](nl/nen-7513/) | §5, the content of a logged event; plus actor role, event content and retention |

## Licensing

The declarations live in `REUSE.toml` and the full terms in `LICENSES/`:

| Identifier | Covers |
|---|---|
| `LicenseRef-EUR-Lex-Reuse` | The EU acts. The Commission's reuse policy, Decision 2011/833/EU, quoted from the EUR-Lex legal notice |
| `CC-BY-4.0` | The consolidated EU texts (each `text.html` pinned at a consolidation), which are additionally the EU's own editorial content |
| `LicenseRef-EDPB-Reuse` | The EDPB guidelines, under the EDPB's own copyright page |
| `LicenseRef-Auteurswet-Art11-Public-Domain` | The Dutch acts, in which no copyright subsists |
| `LicenseRef-UrhG-Para5-Amtliche-Werke` | The German federal acts, which enjoy no copyright protection (§ 5 Abs. 1 UrhG) |
| `LicenseRef-URG-Art5-Nicht-Geschuetzte-Werke` | The Swiss federal acts and their official translations, not protected by copyright (Art. 5 URG) |

The `PROVENANCE.md` and `SHA256SUMS` files, and this index, are the project's
own writing about the vendored material and stay under the project licence.
