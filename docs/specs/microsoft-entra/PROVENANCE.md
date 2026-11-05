# Provenance: Microsoft Entra ID provisioning and identity platform articles

- Source: <https://github.com/MicrosoftDocs/entra-docs>, the repository Microsoft Learn
  publishes the Entra documentation from
- Pin: commit `219d8d57b01779e72e6d5449dc9341334c1fb906` (2026-11-05T13:36:57Z), the default branch's head when
  retrieved, each file checked against the sha256 below
- Retrieved: 2026-11-05
- Articles: `docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md`
  ("Tutorial - Develop a SCIM endpoint for user provisioning to apps from
  Microsoft Entra ID", `ms.date` 09/15/2026) and
  `docs/identity/app-provisioning/customize-application-attributes.md` ("Tutorial - Customize
  Microsoft Entra attribute mappings in Application Provisioning",
  `ms.date` 08/06/2026), with every figure they reference
- Identity platform articles: `docs/identity-platform/reference-microsoft-graph-app-manifest.md`
  ("Understand the app manifest (Microsoft Graph format)", `ms.date`
  02/03/2025, `api.requestedAccessTokenVersion`, with the include file
  `docs/includes/entra-identifier-uri-patterns.md` it embeds),
  `docs/identity-platform/access-token-claims-reference.md` ("Access token claims
  reference", `ms.date` 05/26/2023, `oid`, `idp`, `sub` and the groups
  overage claim) and `docs/identity-platform/optional-claims-reference.md` ("Optional
  claims reference", `ms.date` 07/22/2026, `acct` and `idtyp`); the two
  claims references link no include file and no figure. "Access tokens in
  the Microsoft identity platform" (`docs/identity-platform/access-tokens.md`) is not
  taken: its sample access tokens are refused by GitHub push protection as
  Entra ID tokens, and the manifest reference states the same setting.
  The two claims references are the bytes FerroHEALTH/FerroAUTH vendors in `docs/specs/entra-docs/` at the same commit.
- Upstream licence: the repository states two grants, both recorded here as
  read 2026-11-05. The `README.md` "Legal Notices" section reads:
  "Microsoft and any contributors grant you a license to the Microsoft
  documentation and other content in this repository under the Creative
  Commons Attribution 4.0 International Public License, see the LICENSE
  file, and grant you a license to any code in the repository under the MIT
  License, see the LICENSE-CODE file." The `LICENSE` file it points at
  reads the MIT License ("Copyright (c) Microsoft Corporation. Permission is
  hereby granted, free of charge, to any person obtaining a copy of this
  software and associated documentation files ..."), as does
  `LICENSE-CODE`. Both grants permit redistribution with the notice kept;
  the three files are vendored beside the articles, and the attribution is
  this file: the material is Microsoft Corporation's. `REUSE.toml`
  declares the tree `CC-BY-4.0 OR MIT`, one expression for the two grants.
- Layout: the upstream paths, unchanged
- Fetched by: `scripts/vendor/entra-scim-docs.sh`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `LICENSE` | `c2cfccb812fe482101a8f04597dfc5a9991a6b2748266c47ac91b6a5aae15383` |
| `LICENSE-CODE` | `91f7258203fd43ba6d6f7f72b644eaedee9e3430f035c053beb6f34052f045e2` |
| `README.md` | `32355274e726eb8981b4478aef7917313395ff7ca7be1904488c4adf8b818584` |
| `docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md` | `c413a8dba21bfa7392f571bd9ebd1a42c569a834f4961339648aae5654f6e6b4` |
| `docs/identity/app-provisioning/customize-application-attributes.md` | `326616d9ae71fe04965bb32b34601532b3662adeb1b41e970e94156776d2eb0e` |
| `docs/identity/app-provisioning/media/customize-application-attributes/add-roles.png` | `d2a0a68da5374a1a42a152a07f70a099bafb33795ea7fe2337ce25dfea5c9065` |
| `docs/identity/app-provisioning/media/customize-application-attributes/edit-attribute-approleassignmentscomplex.png` | `e5b15c34d7d19f44e4b872bee7b6bfef257bb0c31cea1e670a198b9d42111ce2` |
| `docs/identity/app-provisioning/media/customize-application-attributes/edit-attribute-assertiveapproleassignmentscomplex.png` | `b1b2e35cf6d16ccdebabc329ea8290b70a1d3c3c3d7be4ee09c4be27a7ef97a4` |
| `docs/identity/app-provisioning/media/customize-application-attributes/edit-attribute-singleapproleassignment.png` | `c094c26b729343f17bf758c960602446837685108cf2d9393b023f50f32d658e` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/oauth2.png` | `e6c3869619792884b5dc703f1acab0b44cecb38f8143d3829cbe25e83f2f778b` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/process.png` | `c2c336d45ed5f6b664f6fb355bbf080a8396ec08351005afaf6da3ee5d5f9d55` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/scim-figure-2b-1.png` | `a28d4e6234cd04b1c5dfa5b91e6dcf37926bae92704ffbc4bd0e25066bfa3cc7` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/scim-figure-3.png` | `8827ba1646381ce95f8fef3c8642c25c417da67a54fd4ffa447611181dd4a825` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/scim-figure-4.png` | `a12bbbe83270e8f10d906ddbbc1a7dd341e231e46815af3d58d86d14095a4bd0` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/scim-figure-5.png` | `aa4ddedc8a9d2d6bb4ceeec64620f9b5f35a98386f13292c47de76f01ca923f7` |
| `docs/identity/app-provisioning/media/use-scim-to-provision-users-and-groups/scim-provisioning-overview.png` | `2d307f01860e79438addfaf6b1c072f7cd66c5e62a50246491d59c579360ff49` |
| `docs/identity-platform/reference-microsoft-graph-app-manifest.md` | `a4738d13921f6c7363d1b9e615f7b00a941289c2cf8584f81c434aa1b80b678f` |
| `docs/includes/entra-identifier-uri-patterns.md` | `ffe8720160b46f32d6e93d5fd1235e93d0ca2f46b18e47c0556b25474f2e7eac` |
| `docs/identity-platform/access-token-claims-reference.md` | `e07447d46ab82957979f12b3caccaf9d07179c6556c7ffbde150d1f754f1e232` |
| `docs/identity-platform/optional-claims-reference.md` | `3b714c57087ee02fbc3ad49ddcd2399a144b2da2bca327b7cd9f7f0a0da797dc` |
