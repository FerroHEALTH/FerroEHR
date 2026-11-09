---
name: ehds-above-openehr
description: Where the EHDS Regulation (EU) 2025/327 conflicts with an openEHR (or other standard's) text, the Regulation wins; never undo EHDS work to turn a conformance row green (owner 2026-11-09)
metadata:
  type: feedback
---

On 2026-11-09 the owner refused, firmly, any suggestion of reverting FerroEHR's committer binding (FerroEHR #5002) to make four Veredictum rows pass: "we are not deleting all our hard work to make our code EHDS ready. EHDS is above openEHR specs." The rows want a client-stated committer merged into AUDIT_DETAILS; EHDS (Regulation (EU) 2025/327) Annex II 3.1 and 3.2 require the authenticated natural person.

**Why:** every FerroHEALTH product is built for the EU EHR market and must meet the Regulation; a standard's text that conflicts with it is reported, not obeyed. The specification stays the oracle everywhere the Regulation is silent.

**How to apply:** never offer "revert the EHDS behaviour" as an option, in any product. A conflict is a recorded deviation with its legal ground (article and annex point) and an upstream report (FerroEHR #5019 is the first); the conformance instrument records it as a regulatory deviation (Veredictum #1074), never as a failure to fix. Related: [[decide-dont-ask]].
