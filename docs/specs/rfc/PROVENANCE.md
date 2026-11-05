# Provenance: IETF RFCs

- Source: the RFC Editor, <https://www.rfc-editor.org/rfc/rfcNNNN.txt>, the text form of each RFC
  as published; an RFC never changes once published, so its number is its
  version
- Retrieved: 2026-11-05
- Pin: the sha256 of each file, in `scripts/vendor/rfcs.sh` and the table
  below; the script refuses any other bytes and any RFC the RFC Editor
  reports obsoleted
- Upstream licence: each RFC's Copyright Notice reads "Copyright (c) <year>
  IETF Trust and the persons identified as the document authors. All rights
  reserved." and "This document is subject to BCP 78 and the IETF Trust's
  Legal Provisions Relating to IETF Documents
  (https://trustee.ietf.org/license-info) in effect on the date of
  publication of this document." Section 3.c of the Trust Legal Provisions
  grants "to each person who wishes to exercise such rights, to the greatest
  extent that it is permitted to do so, a non-exclusive, royalty-free,
  worldwide right and license under all copyrights and rights of authors: i.
  to copy, publish, display and distribute IETF Contributions and IETF
  Documents in full and without modification". RFC 6585 and RFC 6749,
  published in 2012, fall under TLP 4.0 (effective 2009-12-28,
  <https://trustee.ietf.org/license-info/IETF-TLP-4.htm>); the others under
  TLP 5.0 (effective 2015-03-25,
  <https://trustee.ietf.org/documents/trust-legal-provisions/tlp-5/>). Both
  carry the grant in the same words, read 2026-11-05. Each file here is
  the whole document, unmodified, with every notice it carries. The
  Revised BSD licence of section 4 covers only Code Components and is not
  relied on. `LICENSES/LicenseRef-IETF-Trust-Legal-Provisions.txt` holds
  the same record for REUSE.
- Fetched by: `scripts/vendor/rfcs.sh`; never hand-edit, re-run the script
  to update (.claude/rules/vendored-corpora.md)

| Document | Title | Published | Status | Updated by | File | sha256 |
|---|---|---|---|---|---|---|
| RFC 6585 | Additional HTTP Status Codes | April 2012 | PROPOSED STANDARD | none | `rfc6585.txt` | `f6d55d1b491cd515c35827cf9181753b23b2a68c4df14e56d83dc445b3876e58` |
| RFC 6749 | The OAuth 2.0 Authorization Framework | October 2012 | PROPOSED STANDARD | RFC8252, RFC8996, RFC9700 | `rfc6749.txt` | `f204fc8661d6c92d2ec6e0b54808f961a9ad26e792f57f312d9528335519bd71` |
| RFC 7523 | JSON Web Token (JWT) Profile for OAuth 2.0 Client Authentication and Authorization Grants | May 2015 | PROPOSED STANDARD | none | `rfc7523.txt` | `ae24f77a8fc4338903c805c6ace38def1f23d40194aea87b123b13c5b3d2d915` |
| RFC 7642 | System for Cross-domain Identity Management: Definitions, Overview, Concepts, and Requirements | September 2015 | INFORMATIONAL | none | `rfc7642.txt` | `4abbf1bf65926800df6604d627ac0e1d8a0efa784ddcb63b5b59ca8cf4a93e56` |
| RFC 7643 | System for Cross-domain Identity Management: Core Schema | September 2015 | PROPOSED STANDARD | RFC9865, RFC9967 | `rfc7643.txt` | `df799c112a3fa5be3c0fe054c08b1f4eb5d07590c4c8343a014c4537bf638ae3` |
| RFC 7644 | System for Cross-domain Identity Management: Protocol | September 2015 | PROPOSED STANDARD | RFC9865, RFC9967 | `rfc7644.txt` | `b9078e016c98b86934a23c80926b74fc8b59885ab2943729ec2493c17488a09b` |
| RFC 8141 | Uniform Resource Names (URNs) | April 2017 | PROPOSED STANDARD | none | `rfc8141.txt` | `9e3296eaac5641d356f580696894878dddc525124436d1da7005f0748035d41e` |
| RFC 8265 | Preparation, Enforcement, and Comparison of Internationalized Strings Representing Usernames and Passwords | October 2017 | PROPOSED STANDARD | none | `rfc8265.txt` | `ec987e39b242228e870b8d1df836451e19cdb2bd66b1ac105355adeae36ff32f` |
| RFC 8693 | OAuth 2.0 Token Exchange | January 2020 | PROPOSED STANDARD | none | `rfc8693.txt` | `358d658014738bf4dc56c8aae2094ce716ede23b6d5a80488a7dc8e8d72da609` |
| RFC 9068 | JSON Web Token (JWT) Profile for OAuth 2.0 Access Tokens | October 2021 | PROPOSED STANDARD | none | `rfc9068.txt` | `68d3ff3910c0f2340fd7b93b430e00053da845b6eebe095ebef0c8bfda5e14c5` |
| RFC 9449 | OAuth 2.0 Demonstrating Proof of Possession (DPoP) | September 2023 | PROPOSED STANDARD | none | `rfc9449.txt` | `3842c58e1f6043389416023b9bb8d765048266024982fbbd90640e05943f4e13` |
| RFC 9700 | Best Current Practice for OAuth 2.0 Security | January 2025 | BEST CURRENT PRACTICE | none | `rfc9700.txt` | `9919d061d40a97886ca866b51c69389b6c81c65cd4b979056c14fdbebfcf622a` |

An "Updated by" entry names an RFC that changes the document without
replacing it. The updates are not vendored here; a citation that depends on
one names it and vendors it first.
