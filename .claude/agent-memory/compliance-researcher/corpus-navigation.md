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
- CRA: scope Art. 2, defs Art. 3 (no "user", no "secure by default" definition; "remote data processing" = 3(2)), manufacturer Art. 13, reporting Art. 14 (applies from 2026-09-11), dates Art. 69/71; Annex I = id `anx_I` (Part I (2)(a)-(m), Part II (1)-(8)); Annex II = `anx_II`.
- EHDS: Chapter III Arts. 25-49; manufacturer Art. 30; serious incidents Art. 44(7) + def Art. 2(2)(r); Annex II = `anx_II` (3.2 logging); dates Art. 105.
- NIS2: Art. 21(2)(d)+(3) supply chain, rct 85; scope Art. 2(1), 3; Annex I sector 5 Health. Binds Member States, not vendors.
- NL: BW7 ids `Boek7_Titeldeel7_Afdeling5_Artikel457`; Wabvpz 15j -> Begz art. 3 lid 2 (NEN 7510/7512 duty on zorgaanbieder). UAVG art. 4 = territorial.
- DE: StGB § 203 (Abs. 3 S. 2 mitwirkende Personen, Abs. 4); BDSG § 1 Abs. 2 S. 3, § 36 (public bodies only).
- CH export: DSG Art. 16 Abs. 1 + DSV Art. 8 Abs. 1 + Anhang 1 (`annex_1`; lists Deutschland, Finnland, Niederlande).

Extraction traps: the shell's grep is ugrep (long `.{0,200}` patterns fail), so use perl. Dutch/BW HTML wraps sentences across lines, so split on `<div class="artikel"` with -0777. Fedlex ids `art_N`, `annex_1`. EU annex ids `anx_I`/`anx_II`.

Related: [[corpus-coverage-gaps]]
