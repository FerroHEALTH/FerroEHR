---
name: contribution-committer-666-lifecycle
description: Verified 2026-10-06 on commit 5950057ae (#3550/#3590) — the CNF catalogue vs CDR clash on the 666 member lifecycle_state, the 400/422 split inside one UpdateAudit.required list, and how to check CNF/Veredictum payload impact
metadata:
  type: feedback
---

**Check the Veredictum RUNNER, not just the vendored CNF JSON, before ruling a
CONTRIBUTION refusal "CNF-safe".** The vendored CNF robot payloads all carry
committer + lifecycle_state, but Veredictum v0.1.6 builds members in
`app/veredictum/src/exec/driver.rs` (`contribution_member`,
`attestation_member`): it fills committer on every audit (so removing the
committer fallback is catalogue-safe) but builds 666 members WITHOUT
`lifecycle_state` and its authoring guard REFUSES one. So requiring
lifecycle_state on 666 (SM master03 §Version Update Semantics L25 "must be
supplied in all cases") flips 4 passing positive attestation cases red and lets
4 negatives (validation_failed has `alt_status: [400]` on commit_contribution)
pass for the wrong reason. Read the remote at the pinned tag via a scratchpad
shallow clone (never the local cache).

**Why:** the catalogue argues the 666 member shape from RM master06
§Contributions (inference: no version → no lifecycle) and never cites SM
master03 L25; explicit SM sentence + OAS UpdateVersion.required beat the
inference, but the same "UpdateVersion lists it under required" argument would
also demand `data`, which the CDR 422s on a 666 member — cite SM L25 alone.

**How to apply:** for any CONTRIBUTION envelope rule change, (1) diff the
runner's member builder, (2) list catalogue cases + `docs/conformance/ferroehr/results.json`
verdicts, (3) check `alt_status` on the binding for masking. Also recurring:
one `UpdateAudit.required` list answered with two statuses (absent committer
400, absent envelope change_type 422 — `contribution.rs` parse_contribution_audit);
demographic `contribution_create` served OAS declares no 422 though the shared
`commit_version_set` emits it. AMB-90 already registers the master06 copy-down
vs per-member-committer tension (upstream #1536) — do not re-file.
