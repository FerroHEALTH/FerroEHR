# Provenance: the Dutch role and organisation type code lists

The Wabvpz Art. 15f check of a federation request under an NL agreement reads
these lists (`app/ferroehr-rest/src/extensions/access/authn/wabvpz.rs`).

## Nictiz zib 2020 package (committed)

- Source: <https://packages.fhir.org/nictiz.fhir.nl.r4.zib2020/0.12.1-beta.1>, the FHIR package registry's copy of Nictiz's
  package `nictiz.fhir.nl.r4.zib2020`; every published version of it is a beta, and this is
  the newest on 2026-11-08
- Pin: version `0.12.1-beta.1`, tarball sha256 `c4fc5561cd592c8dc7760a5674c2c7998334f6197821096155ce357d2e7a0a8a`, each file
  below pinned by its own sha256
- Upstream licence: CC0 1.0 (`CC0-1.0`, the `license` of the package
  manifest, `package.json`, vendored here;
  <https://creativecommons.org/publicdomain/zero/1.0/>). The package ships no
  licence file of its own.
- What is taken: the `organization-type` CodeSystem
  (`http://nictiz.nl/fhir/NamingSystem/organization-type`, OID
  2.16.840.1.113883.2.4.15.1060) and the `OrganisatieTypeCodelijst` ValueSet
  (2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3, 2020-09-01), the care provider
  types of the zib Zorgaanbieder
- Cache only: the `uzi-rolcode` CodeSystem
  (`http://fhir.nl/fhir/NamingSystem/uzi-rolcode`, version 11.7.0, OID
  2.16.840.1.113883.2.4.15.111). Its codes and titles reproduce Bijlage 3 of
  the UZI-register CPS below, which the package's CC0 cannot license, so the
  script writes it into the git-ignored `.vendor-cache/nl-role-codes/`, the
  Wabvpz tests read it there, and this repository carries none of it
- Layout: the file names inside the package's `package/` directory,
  unchanged

## UZI-register CPS v11.7 (cache only)

- Source: the UZI-register publication page <https://www.uziregister.nl/documenten/publicaties/2024/07/29/cps-uzi-register-v11.7>, downloaded from
  <https://www.uziregister.nl/site/binaries/site-content/collections/documents/2024/07/29/cps-uzi-register-v11.7/29-01-24+RK1+CPS+UZI-register+V11.7.pdf>
- Document: Certificate Practice Statement UZI-register, version 11.7,
  23-01-2024; Bijlage 3 "Beroepstitels, opleidingstitels en specialismen"
  lists the role codes, 01.008 Bedrijfsarts and 01.048 Verzekeringsarts among
  them
- Retrieved: 2026-11-08, checked against the sha256 below
- Upstream licence: none. The document reads "Copyright CIBG 2024© te Den
  Haag" and forbids reproduction or publication without CIBG's prior written
  permission, so the script fetches it into the git-ignored
  `.vendor-cache/nl-role-codes/` and this repository carries none of its
  content.

- Fetched by: `scripts/vendor/nl-role-codes.sh`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| `package.json` | `b57d7f94a7400c07365959e5b245483acd5c4f2e8bd444e30de9e17e8a18e116` |
| `CodeSystem-RoleCodeNLZorgaanbiederType-organization-type.json` | `f6b21a531b28dc7feccfb5b1381ab877c831dc4f9bd42dec5154c140b7c0fa1e` |
| `ValueSet-OrganisatieTypeCodelijst-2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3--20200901000000.json` | `838a64beb99897854ffab8882b9fe9d9d204f80ea36320bf907e4e55030b0786` |

| File | sha256 | Status |
|---|---|---|
| `CodeSystem-RoleCodeNLZorgverlenertypen-uzi-rolcode.json` | `676b3edb095f375a35f12a65f828cd74903f71fe4dfb4e7a1aebe9b58901805e` | cache only |
| `29-01-24+RK1+CPS+UZI-register+V11.7.pdf` | `dc9889a3d592a14409850d53bda9ddf1c61398e6873d40d78cfa0de47086d4a1` | cache only |
