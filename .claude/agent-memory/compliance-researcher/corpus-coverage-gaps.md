---
name: corpus-coverage-gaps
description: What the vendored law corpus does NOT carry (GDPR recitals, ePrivacy, Tw, NEN text, NIS2 transpositions, Land/cantonal law) and the CRA/EHDS amendment gap
metadata:
  type: reference
---

Verified 2026-10-05 against `docs/law/`; re-check if a vendor script re-pins.

- **GDPR has no recitals**: `eu/gdpr/text.html` is consolidation 02016R0679-20160504, zero `id="rct_"` anchors (CRA/EHDS/NIS2 OJ texts do carry recitals). Recitals 14/26/30/47/63 cannot be cited from the corpus; answer from Arts. 1, 2, 4(1) and say so.
- **Not vendored at all**: Directive 2002/58/EC (only cross-refs: GDPR Art. 21(5), 95; CRA rct 72; EHDS Art. 1(3); NIS2 Art. 2(12), 46); Dutch Telecommunicatiewet (incl. 11.7a); NIS2 national transpositions; German Land law; Swiss cantonal law.
- **NEN 7510/7512/7513**: PROVENANCE records only, no text.
- **CRA is the unconsolidated OJ text**: EHDS Art. 104 (ehds `id="art_104"`) replaces CRA Art. 13(4), 31(3), inserts 32(5a) for EHR systems; not in `eu/cra/text.html`.
- **CRA support acts not vendored**: no Art 27 harmonised standards or common specifications, no Art 13(24) SBOM-format implementing act, no ADCO support-period guidance (verified 2026-10-06). Vendored CRA acts: 2025/2392 (categories), 2026/881 (dissemination delay), 2025/1535 (vehicle exclusion).
- **EDPB 01/2025** is the only corpus text mentioning IP addresses (Example 7, printed p. 41); non-binding.

Related: [[corpus-navigation]]
