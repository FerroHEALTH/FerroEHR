# Provenance: the FHIR R4 and R5 Consent resource

- Source: the HL7 FHIR specification, <https://hl7.org/fhir/R4/> (FHIR R4, 4.0.1) and
  <https://hl7.org/fhir/R5/> (FHIR R5, 5.0.0), the published releases
- Pin: each file by its sha256 below; a release file does not change once
  published, and the script refuses any other bytes
- Document: the `Consent` resource of each release: the resource page
  (scope, boundaries, background, the element table, the constraints), its
  formal definitions, its StructureDefinition (`consent.profile.json`), the
  value set pages of the bindings the ABAC design maps (R4 `Consent.scope`,
  and in both releases `category`, `status`, `provision.type` or
  `decision`, `provision.data.meaning`, `provision.action` and the policy
  codes of R4 `policyRule` and R5 `regulatoryBasis`), the examples index
  and every published Consent example in JSON, and the licence page of each
  release
- Retrieved: 2026-11-13
- Upstream licence: read for this copy on 2026-11-13. Each release's
  `license.html`, vendored here, states that the specification is licensed
  under Creative Commons "No Rights Reserved" (CC0 1.0). HL7 keeps the
  "FHIR" trademark, which this copy does not use as a mark.
  `REUSE.toml` declares the tree `CC0-1.0`.
- What is taken: the files above, each whole, without the stylesheets,
  scripts and images the pages link. The XML and Turtle forms of the
  examples, the mappings page and the large content-class and content-code
  value sets are not taken.
- Layout: the release directory, then the file name, as the specification
  serves them
- Fetched by: `scripts/vendor/fhir-consent.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `R4/consent.html` | `438c1e3de0759274f2196fa3bb3efd0f56f812543b3a4c7547e86b42f26c6d2c` |
| `R4/consent-definitions.html` | `9ddabeb5c4821b9e923dd767aae1106e9daa3a81a56112f9de9be68d9b42507e` |
| `R4/consent.profile.json` | `c4d00dcd93cfc55242c315f72dfe5a4f25f4a590f162bf20ccad16145972b02d` |
| `R4/consent-examples.html` | `afcb4440e95d71c310e52bd65d3306e1bc8f332d31cce40cd204dc4fdb5b4c47` |
| `R4/license.html` | `b2aeb02247ca2aa855ed1fce29f62600aeeb8dcc8257ec1e0b40ceaf4fd60a15` |
| `R4/valueset-consent-scope.html` | `e0ed4a9ceb5c3a3e8b08c5d01fafe7927af5f5b530e90bfe1dff1cb986c3eb9e` |
| `R4/valueset-consent-category.html` | `abef5c333602abd495aea3c2f6b9bf0af41138a1f3edf2ff2cc849522e86c809` |
| `R4/valueset-consent-state-codes.html` | `28d193618ffaa5cc1651e5ce8a4ccece7e3eb71fff2e001e373a516aa8e2f75c` |
| `R4/valueset-consent-provision-type.html` | `5690d59a40c30335f2669da6790a730c283859a69b3ddf365f4ab33c799ca9cb` |
| `R4/valueset-consent-data-meaning.html` | `24c717e06ab1e6f46b3335b91a6029383ba44eb19d2742b99a5506afdf76bbf0` |
| `R4/valueset-consent-action.html` | `517798619f2f9b72b391b938ae499cdecfd14585cf25f8e6a6fa180215769ed5` |
| `R4/valueset-consent-policy.html` | `8106d77f42fbaea38af9ee15a7b7b38a4e77e92318507e162fae9bc1da9d3841` |
| `R4/consent-example.json` | `464abff4f0bda68b6247e17acb5bad76ab9d1701e20b68357b67bdace62c6924` |
| `R4/consent-example-Emergency.json` | `f13c1ff607d9b120f0b18cb8f9c880e63be55c0df18965d4d8db554218a06868` |
| `R4/consent-example-Out.json` | `eaf2721c7e37e664aca46ea327c504e62213b87845668c978eec5a5535122d2e` |
| `R4/consent-example-grantor.json` | `7e5fd4946f33c84555f8f7878623bfd7bc65efd5e3da29f3aad324c213fa4573` |
| `R4/consent-example-notAuthor.json` | `b3be9411b1fe9ec38a82bbdcf075076abb9c4053c746088328b472dc39b2f00d` |
| `R4/consent-example-notOrg.json` | `d5ce88400d638badd7c9eb2bb63d637644499750cf02343c0cdce71a6f68c24d` |
| `R4/consent-example-notThem.json` | `c1fe61d49df3053ac0e1e074d0d2b86b79dff5a9eb797b46eeab1f8c78ea77fb` |
| `R4/consent-example-notThis.json` | `e1ac7fa8868d4928ed596ebe7a39ad83697f878ed04c15e12d5e9df43397dff7` |
| `R4/consent-example-notTime.json` | `f29b02ce05901311a4112aae201384d62307489e26ce38dff9602a55c1fdebe8` |
| `R4/consent-example-pkb.json` | `3369a161d5e0e819335c13cf538d60dde6cc3d2193c31592c405dfa63754dfd4` |
| `R4/consent-example-signature.json` | `90da67cb25f3b7dadbaee177e9d1af6fda6f7ab1f08eaeac649ae85a989abf42` |
| `R4/consent-example-smartonfhir.json` | `0834eccf9de9b717d3cba6b807090fe9420946e5a60aa7b1f7c64c2deb2414ea` |
| `R5/consent.html` | `9cefa81d1a6885d0d466330104d0441b88e5e51fc5bd011a03be2c9ddf8ea25b` |
| `R5/consent-definitions.html` | `7c42c4128cf540f3691ff4711a06309911e53ba2bd6fbffab38ccafd53f3b747` |
| `R5/consent.profile.json` | `d647d01f64c6c3015b2efd85ad0f3f5d63f484cb9ee746e273f4fd88ae55c2ec` |
| `R5/consent-examples.html` | `362cbe8dc01644ae36731283c221087cf09ad7e059df53a03b1514f76e37dfa9` |
| `R5/license.html` | `452f8651729bb0e6b93c75a22f772cb7ece300a3585e1da17be944964b44aa90` |
| `R5/valueset-consent-category.html` | `64013a47b5341cc90658d6e87871ea0f8a91e49666161da0f9e0b239186d0df0` |
| `R5/valueset-consent-state-codes.html` | `d46d4b1e2b0af90acc671504e82d364ccf6797e0c348f100254557a75b77fac1` |
| `R5/valueset-consent-provision-type.html` | `bd78dee7d35db1aa5e92d7d5f7df67978e3b431f56761081b4a004386d38ff18` |
| `R5/valueset-consent-data-meaning.html` | `b39a1731d2fde3f5bbc8cb9aa7bb18d7fb5d867cbcb791137707aaf5b462101e` |
| `R5/valueset-consent-action.html` | `d1f017ed931997a9a0841cdef307fbe698458ec581ec416095677690ab89afa1` |
| `R5/valueset-consent-policy.html` | `aac412ed8b2398375bb20661b05724a28dab67f0008072ba6ceb76c1893648ae` |
| `R5/consent-example.json` | `d96a58b9322ddab78a09ac36a0cfc72a8b82240c5da9f137c53c3ca786b7326c` |
| `R5/consent-example-CDA.json` | `7d1e951f65b73cec42f11c93f1d01b35f585d8a0e1b0115dde6ac4f11698fc2a` |
| `R5/consent-example-Emergency.json` | `71c2a15953b96b250df93e01edee963de568d7ce917785f6221a125791f994e8` |
| `R5/consent-example-Out.json` | `ae16b1c3abeeafde2d53c91f06f6cbb57a498c5dbaa072a50ab9f620be8c0332` |
| `R5/consent-example-grantor.json` | `da308f4d572097f91adeae4ca050f1b1fc05c52c2bdf26cbab37224016bc92bf` |
| `R5/consent-example-notAuthor.json` | `f3dcd222b3844598fcf01f30e94a7e14d2c1db78af8268baf1a62bba6ee5e112` |
| `R5/consent-example-notOrg.json` | `31c897a58f9f4ab34a57ada0eb2f69a4fec6b872a37f01c0f6e9e951c24d0ddb` |
| `R5/consent-example-notThem.json` | `112b527688c76cbad2374c256fd2078937937dc812eea954e353df3a74a74687` |
| `R5/consent-example-notThis.json` | `9d58d17649601d7dcfb84666d195f5cfd65c04b30c7911679189e80abf4492b3` |
| `R5/consent-example-notTime.json` | `8544d313e37f1fb146e1010bcd1e77257b4f346e527b53aeb944b7b31a6c4f6f` |
| `R5/consent-example-pkb.json` | `1f0add6432a0507fec7bfadeab74b7ee18ef776482e160270e5be0e805bb746f` |
| `R5/consent-example-smartonfhir.json` | `b2dc4e962faa388dbcd5debdfd7d15e210079eee79be7ab99da5dee75f059ef0` |
