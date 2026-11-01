---
name: version-signature-location
description: Where VERSION.signature / digital-signature / canonical_form semantics live in the RM spec + CNF, and the server-vs-client signing question
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

VERSION digital signature topic ownership:

- Prose semantics: `RM/docs/common/master06-change_control_package.adoc`
  §"Digital Signature" (also §"Attestation" for the ATTESTATION coupling).
  Signing is permissive ("a digital signature ... CAN be made"), openPGP/RFC4880,
  over the canonical serial form with the signature attribute Void. Exact
  serialisation is marked *To Be Determined* (openEHR has not fixed it; ODIN vs XML).
- Class tables: `RM/docs/UML/classes/org.openehr.rm.common.version.adoc` —
  `signature` is `0..1` (optional) String; `canonical_form()` function =
  "serialising all attributes except signature". ORIGINAL_VERSION table:
  `org.openehr.rm.common.original_version.adoc`.
- `time_committed` server-compute rule: master06 (~line 90) — Version/Contribution
  audit `time_committed` "should be computed on the server". commit_audit is part
  of the VERSION object, so canonical_form includes it.
- Attestation signature (separate from VERSION.signature): `RM/docs/common/master04-generic_package.adoc` (ATTESTATION.proof, openPGP).
- SM (`SM/docs/`) and ITS-REST (`ITS-REST/docs/`) have NO digital-signature
  service/wire obligation — the only "Signature" hits in SM are the UML method
  "Signature" column header; ITS-REST "signature" hits are all JWS/SMART auth.
- CNF: no test case exercises VERSION.signature/digest generation.
