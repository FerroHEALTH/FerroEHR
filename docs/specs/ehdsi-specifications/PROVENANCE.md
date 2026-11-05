# Provenance: the eHDSI documents the party binding cites

- Source: the eHDSI wiki of DG SANTE, the Directorate-General for Health and
  Food Safety of the European Commission, which publishes them for
  MyHealth@EU, at
  <https://webgate.ec.europa.eu/fpfis/wikis/display/EHDSI/2.+eHDSI+INTEROPERABILITY+SPECIFICATIONS%2C+Requirements+and+Frameworks>. The wiki sits behind EU Login, so the owner retrieved each
  document and the script copies it verbatim from the owner's directory,
  finding it by its sha256; it downloads nothing.
- Retrieved: 2026-11-05, by the owner
- Documents, with the version and date each control page states:
  - `ehdsi-interoperability-specifications-8.0.1.docx`: eHDSI
    Interoperability Specifications, document version 8.0.1, 24/02/2025,
    Wave 8 Operation Ready (supplied as
    `eHDSI_Interoperability_Specifications_v8.0.1.docx`)
  - `ehdsi-saml-profile-10.0.0.pdf`: SAML Profile, document version
    10.0.0, 01/06/2026, Wave 10 Operation Ready
  - `ehdsi-audit-trail-profile-10.1.0.docx`: Audit Trail Profile, document
    version 10.1.0, 05/10/2026, Wave 10 Operation Ready
  - `ehdsi-section-i-security-policies-10.0.0.pdf`: Section I Security
    Policies, document version 10.0.0, 01/06/2026, Wave 10 Operation Ready
  - `ehdsi-section-ii-security-services-10.0.0.pdf`: Section II Security
    Services, document version 10.0.0, 01/06/2026, Wave 10 Operation Ready
  - `ehdsi-xca-profile-8.0.1.pdf`: XCA Profile, document version 8.0.1,
    24/02/2025, Wave 8 Operation Ready
- Upstream licence: each cover and control page states "Reuse is
  authorised, provided the source is acknowledged." and "Distribution
  level*: PU", where "PU = Public", read 2026-11-05. The acknowledgement is
  this file: the documents are DG SANTE's, and each cover's source line
  ("Source: DG SANTE, CEF eHealth DSI, <year>" or "Source: DG SANTE,
  MyHealth@EU, <year>") names it. `REUSE.toml` declares the tree
  `LicenseRef-eHDSI-Reuse`.
- Metadata: several files carry a stale title in their document properties
  ("D3.7.2 Section I - Security Policies_v0.8"), so the sha256 identifies
  each file, never the metadata.
- Identity: the same sha256 as FerroHEALTH/FerroFED records for its copy in
  `docs/specs/ehdsi-specifications/`
- Fetched by: `scripts/vendor/ehdsi-specifications.sh DIR`; never
  hand-edit, re-run the script to update (.claude/rules/vendored-corpora.md)

## Read and not redistributed

The MyHealth@EU Requirements Catalogue, version 10.0.0 OR (W10 Operation
Ready, status date 06 Mar 2026), supplied as
`eHDSI Requirements Catalogue v10.0.0 OR.pdf`, carries no reuse statement,
no distribution level and no licence. The script checks the owner's copy
against the sha256 below and copies it nowhere: no part of its text enters
this repository, and a citation names it by version, section and
requirement id only.

| File | sha256 | Status |
|---|---|---|
| `myhealth-eu-requirements-catalogue-10.0.0.pdf` | `4743869bbd57564a81b567c86619523f73f2ff0fabfec9a12f78127fd128408a` | not redistributed |

## Committed

| File | sha256 |
|---|---|
| `ehdsi-interoperability-specifications-8.0.1.docx` | `e4b79689176b8547ae0987f065ee0e1dce7734cb111b95c6fda75f33fd6230c1` |
| `ehdsi-saml-profile-10.0.0.pdf` | `edd17e79b49ec56cd3799a58fd392ef51a09aa53c4bc726a05b4252dd7c2501d` |
| `ehdsi-audit-trail-profile-10.1.0.docx` | `8cbc4edbc39071d7fdad09e1d913a83f1585e3a41009ece3b63942b2fba146e3` |
| `ehdsi-section-i-security-policies-10.0.0.pdf` | `010bed197540bdcd92574a0df4824d21d7ca615fa5a905dea469a7cc70c89167` |
| `ehdsi-section-ii-security-services-10.0.0.pdf` | `0faf5f8895b0b500ea4e3f46e5de21a3c048e4c38939989e73483ab763c7c06d` |
| `ehdsi-xca-profile-8.0.1.pdf` | `f76eebf01e10d719d015dd4bba943d2eb3af915f8402795479c8a8ed8d21c79e` |
