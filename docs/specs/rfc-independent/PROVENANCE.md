# Provenance: Independent Submission RFCs, cache only

**This directory holds this record and nothing else.** The documents are read
from the cache and never committed.

- Source: the RFC Editor, <https://www.rfc-editor.org/rfc/rfcNNNN.txt>
- Retrieved: 2026-11-07, each checked against the sha256 below; the script
  refuses an RFC the RFC Editor reports obsoleted or moved out of the
  Independent Submission Stream

| Document | Title | Published |
|---|---|---|
| RFC 3881 | Security Audit and Access Accountability Message XML Data Definitions for Healthcare Applications | October 2004 |
| RFC 8785 | JSON Canonicalization Scheme (JCS) | June 2020 |

- Upstream licence: RFC 3881 reads "Copyright (C) The Internet Society
  (2004). This document is subject to the rights, licenses and restrictions
  contained in BCP 78"; RFC 8785 is "subject to BCP 78 and the IETF Trust's
  Legal Provisions Relating to IETF Documents". The Trust Legal Provisions 5.0
  grant third parties the right to copy and distribute IETF Documents in full
  (section 3.c.i) and state that section 3.c does not apply to the Alternate
  Streams, the Independent Submission Stream among them (section 8.c.iii),
  read 2026-11-07 at
  <https://trustee.ietf.org/documents/trust-legal-provisions/tlp-5/>. With no
  grant that reaches these documents, the script fetches them into the
  git-ignored `.vendor-cache/rfc-independent/` and this repository carries
  none of their content.
- Fetched by: `scripts/vendor/rfcs-independent.sh`; never add a file here,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 | Status |
|---|---|---|
| `rfc3881.txt` | `3dea919cf0b565890a47089560a2f22dbbb9ba1af34442c56cee9372fd7b4308` | cache only |
| `rfc8785.txt` | `63d52294eb0e3f0014174288186d388b4ddbf2c67d1ce8af1d9726eb0c3ab240` | cache only |
