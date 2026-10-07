---
name: corpus-navigation
description: Where recurring topics live in docs/law (personal-data definition, defaults, objection, CRA Annex I/II, EHDS Annex II, NIS2 supply chain, § 203, BW 7:457, DSG export) plus file-format extraction traps
metadata:
  type: reference
---

Topic -> provision (read the text again before citing; this is navigation only):
- Personal data / natural persons: GDPR Art. 1(1)-(2), 2(1), 4(1); DSG Art. 2 Abs. 1 + Art. 5 lit. a (DSG is explicit: natural persons only).
- Roles: GDPR Art. 4(7)-(10), 26(1), 28(10), 29. Health: 4(15), 9(1)-(3).
- Defaults: GDPR Art. 25(2); DSG Art. 7 Abs. 3. Objection: GDPR 21(1),(4),(5) + 17(1)(c); DSG Art. 30 Abs. 2 lit. b / 31 Abs. 1. Records exemption trap: GDPR 30(5) ("not occasional").
- CRA: scope Art. 2, defs Art. 3 (no "user", "secure by default", "known" or "original state" definition; (1) product, (2) remote data processing, (13) manufacturer, (20) support period, (23) intended purpose, (39) SBOM, (40)-(42) vulnerability/exploitable/actively exploited, (48) FOSS), Art 6 (a)=Part I on product, (b)=Part II on manufacturer processes, Art 7(1) core functionality decides class (7(2) criteria guide the Commission, they classify nothing), manufacturer Art 13 (25 paras; 13(24) Commission, 13(25) ADCO/MSA), reporting Art 14 (from 2026-09-11), Art 32 routes, dates Art 69/71; Annex I = `anx_I` (Part I (2) chapeau is conditional on the Art 13(2) risk assessment), Annex II user info = `anx_II` (points 1-9, 8(a)-(f)), Annex III/IV.
- CRA IR 2025/2392 (`eu/cra-product-categories-2025-2392`): ids art_1-3, anx_I (Class I 1-19, Class II 1-4 technical descriptions), anx_II (critical, hardware only); recitals (3)-(5) = core functionality vs integrated/ancillary functions.
- CRA for EHR systems: EHDS Art 104 inserts CRA Art 32(5a) (conformity via EHDS Chapter III), replaces 13(4), 31(3); EHDS rct (112) = CRA via EHDS framework, testing environments not applied, SaaS outside CRA; rct (38) = general-purpose middleware/DBMS not an EHR system; EHDS Art 2(2)(k) EHR system def, Art 30 manufacturer, Art 39 single DoC, Art 40 testing env.
- EHDS: Chapter III Arts. 25-49; manufacturer Art. 30; serious incidents Art. 44(7) + def Art. 2(2)(r); Annex II = `anx_II` (3.2 logging); dates Art. 105.
- NIS2: Art. 21(2)(d)+(3) supply chain, rct 85; scope Art. 2(1), 3; Annex I sector 5 Health. Binds Member States, not vendors.
- NL: BW7 ids `Boek7_Titeldeel7_Afdeling5_Artikel457`; Wabvpz 15j -> Begz art. 3 lid 2 (NEN 7510/7512 duty on zorgaanbieder). UAVG art. 4 = territorial.
- DE: StGB § 203 (Abs. 3 S. 2 mitwirkende Personen, Abs. 4); BDSG § 1 Abs. 2 S. 3, § 36 (public bodies only).
- CH export: DSG Art. 16 Abs. 1 + DSV Art. 8 Abs. 1 + Anhang 1 (`annex_1`; lists Deutschland, Finnland, Niederlande).

Extraction traps: the shell's grep is ugrep (long `.{0,200}` patterns fail), so use perl. Dutch/BW HTML wraps sentences across lines, so split on `<div class="artikel"` with -0777. Fedlex ids `art_N`, `annex_1`. EU annex ids `anx_I`/`anx_II`.

Related: [[corpus-coverage-gaps]]
