---
name: ferroehr-v6-major
description: FerroEHR v6.0.0 is a major release where everything is done the clean, proper, new way with no legacy (owner, 2026-11-16)
metadata:
  type: project
---

FerroEHR v6.0.0 is the breaking major: the access-control and access-log
redesign, the core that parses once and renders once, and the other breaking
changes. It is the release where everything gets a proper clean design, with
no legacy. Owner, 2026-11-16: "this is the moment to do everything clean and
propper".

**Why:** a major is the moment a breaking cleanup costs nothing extra.

**How to apply:** prefer the clean breaking design over compatibility, fold
the cleanups that belong to a major into v6.0.0, and check every older issue
for relevance before it moves in ([[verify-before-implementing-old-issues]]).
Related: [[new-design-no-legacy]], [[no-half-solutions]].
