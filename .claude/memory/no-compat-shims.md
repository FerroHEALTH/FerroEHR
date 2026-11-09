---
name: no-compat-shims
description: Family hard rule. Refactors break APIs freely and leave nothing of the old shape; a removal aid (a named refusal of a removed key, flag, env variable, route or format) lives exactly one release, then is deleted fully; no legacy support ever (owner 2026-11-01, 2026-11-09)
metadata:
  type: feedback
---

A refactor in any FerroHEALTH repository breaks internal and crate APIs as far as it needs to and leaves no trace of the old shape: no `#[deprecated]` item, no alias or re-export kept for old callers, no shim, wrapper, stub or `todo!()` placeholder, no "legacy" module. The removed item goes, and every caller in the workspace moves in the same pull request. Only a published wire contract (the specification the product serves) stays fixed. The work runs bottom-up: shared crates first, each change fixing its callers, then the application layer.

**Why:** the owner, 2026-11-01, during the FerroEHR #4212 and #4663 clean-up: "it's very very important that we need to make breaking changes", "we should not have legacy or backwards compatibility things", "I do not want stubs as well". The family's crates are pre-1.0 and versioned at publish, so a break costs a version bump and nothing more ([[no-half-solutions]]).

**One release, then deleted (owner hard rule, 2026-11-09, every FerroHEALTH product):** a removal aid lives for exactly one release. That covers a named refusal of a removed or renamed config key, a "renamed to" message, and every other trace of an old shape: an old CLI flag, a removed route's refusal, an old env variable, an old file-format reader. The release that removes or renames the thing ships the refusal and the upgrade step in its changelog fragment. The next release deletes the refusal completely; from then on `deny_unknown_fields`, or the plain absence of the route or flag, refuses the old form like any other unknown input. Nothing that serves an old shape is kept beyond that one release. The owner: "we keep this removed and mapped to only for one version and then in the next version you need to delete it fully so we keep always an clean codebase, otherwise some impl will only grow and grow with deleted or removed keys", and "this is not only for the config but also for other things!!! we should never support legacy".

**How to apply:** every brief that removes or renames something says the refusal lives one release only. Each refusal records the release that introduced it (a `// NOTE:` or its row in the refusal table), so the next cut can find it. The release PR, or the first PR after a release, deletes every refusal the previous release introduced. A refusal older than one release found anywhere is deleted on sight, with a changelog entry under Removed.

**FerroEHR** is released but has no production installs ([[ferroehr-no-production-installs]]). A removed config key fails at boot through the strict sweep (`.claude/rules/configuration.md`); a named refusal, where one is added, lives the one release. The rewrite rules of [[rewrite-breaks-everything-shipped-means-released]] still hold.
