# Provenance: the IHE Basic Audit Log Patterns implementation guide

- Source: <https://github.com/IHE/ITI.BasicAudit>, rendered at
  <https://profiles.ihe.net/ITI/BALP/index.html>
- Pin: tag `1.1.4`, which resolves to commit `15309210301cbcafb5657d0f5c74214570171b8c`, each file
  checked against the sha256 below
- Document: Basic Audit Log Patterns (BALP), Release 1.1.4, IHE ITI Technical
  Framework Supplement, Trial Implementation: Volume 1 (`volume-1.md`, the
  profile, numbered 3:5.7 in the content section), the content profiles
  (`content.md`, §3:5.7.1 to §3:5.7.7) and the FHIR Shorthand of the
  `AuditEvent` profiles the openEHR access log design maps to (Create, Read,
  Update, Delete, Query and their Patient variants, the OAuth and SAML access
  token patterns, the consent decision and the privacy disclosure)
- Retrieved: 2026-11-07
- Upstream licence: read for this copy on 2026-11-07. The repository's
  `LICENSE` is the Creative Commons Attribution 4.0 International Public
  License, vendored beside this file. The IHE Technical Frameworks General
  Introduction, chapter 9 (<https://profiles.ihe.net/GeneralIntro/ch-9.html>),
  also grants every user a royalty-free licence to reproduce and distribute
  IHE Technical Documents. The attribution is this file: the guide is IHE
  International's. `REUSE.toml` declares the tree `CC-BY-4.0`.
- What is taken: the three narrative pages above, the licence, the
  thirteen FHIR Shorthand files that define the profiles and the code system
  they use, and `sushi-config.yaml`, which states the guide's FHIR version
  (4.0.1). The examples, the capability statement, the test plan and the
  build scripts are not taken.
- Layout: the upstream paths, unchanged
- Fetched by: `scripts/vendor/ihe-balp.sh`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `LICENSE` | `9e5f1b3c610b9c2da5c313bf81d577a7d1acec686bdb0384edefa6df0f90cd94` |
| `input/fsh/Aliases.fsh` | `97d190fe26b35a46d9a6b9547c0f8955dfe3fd8368a708e7689906faabc6aed6` |
| `input/fsh/AuthZ.fsh` | `07d019b4b7fbc7739fec2d7b3f25d16358a83d3ab6f3d713957a5fd59c051148` |
| `input/fsh/Disclose.fsh` | `b77e4e2f23da32c660cfe25e190676fa624a52940144248df4f11f482c5cf1c8` |
| `input/fsh/IUAaccessToken.fsh` | `a8503917d34c1fcb2acd96cb9694886bd495e9ab094c1d823a0798ba26eaaaf0` |
| `input/fsh/PatientCreate.fsh` | `9d8be2ac1cfd2a980490637b0da0a2212cfd9fc6c24c13ad0cdc68c430d4d1ce` |
| `input/fsh/PatientDelete.fsh` | `3aca03589b4e280a92df3bea9b47f3de8395bdc74043aaf3d6b2048c9019ddf3` |
| `input/fsh/PatientQuery.fsh` | `a7522860afc5535e8701ff4c708bb08f743c683a79dcbcc244e436d7a38ca9fa` |
| `input/fsh/PatientRead.fsh` | `67d830872f8cc91a0c9d65ec8018bbbdf8609e1e0b7ff359f1c88a69b4189682` |
| `input/fsh/PatientUpdate.fsh` | `bf742c4d3e43c1660834dec70a2a7c438a1a68de134da97a572b0f77a31dc940` |
| `input/fsh/SAMLaccessToken.fsh` | `b8eabdc7eb7476822198711fcae81363bf259791e95c6c8e7fa4cb598171c13c` |
| `input/fsh/terms.fsh` | `ce6507a0d2698a3b83597d3a38a1d7c374ef364368e18d53084b436707f5c073` |
| `input/pagecontent/content.md` | `176c808638dbe23dffc21babf5907e0a8e10b8ac035a2d928646d0217e53d142` |
| `input/pagecontent/index.md` | `760a871e3f83f9535a848b44f48de4842b9e9d0b1394452b3ccae80a5788153c` |
| `input/pagecontent/volume-1.md` | `2389dcd18182cbd996a722017d4cd03cc61d1288126964aa84143499f2edd020` |
| `sushi-config.yaml` | `a1e00a30875d54df761fc80c8c50b2d770a1c99293e9f59a0357497b6b70f806` |
