---
name: veredictum-other-session
description: "Veredictum is worked in another session and is being rewritten, so its verdicts are not authority; read it ONLY from the official remote repo, never the local checkout or the ~/.cache/ferroehr-veredictum cache"
metadata:
  type: feedback
---

Owner 2026-08-27, reinforced angrily 2026-08-31: never touch the Veredictum checkout, tracker state, or dispatches from the FerroEHR session, and never read from the pipeline's local cache (`~/.cache/ferroehr-veredictum/<pin>/repo`) either. Every Veredictum read (register entries, catalogue cases, party material, release notes) goes to the official repo `github.com/Vernum-Projecten/Veredictum` at the pinned tag, via `gh api` or raw URLs.

Owner 2026-10-25 ([[family-architecture]]): Veredictum is being rewritten and is out of scope until the owner says otherwise, so its verdicts are not authority. A red or green row is evidence to adjudicate against the vendored openEHR text, never the reference.

**Why:** the cache is the pipeline's private mechanism and may be stale, half-written, or mid-mutation by the conformance scripts; the other session owns the working checkout. The remote tag is the only immutable read surface.

**How to apply:** `gh api repos/Vernum-Projecten/Veredictum/contents/<path>?ref=v<pin>` (or the raw URL at the tag) for any file; `gh issue view N --repo Vernum-Projecten/Veredictum` for tracker context. FerroEHR-side pin and integration work stays fair game. See [[session-workflow-gotchas]].
