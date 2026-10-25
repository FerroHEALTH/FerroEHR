---
name: contribution-licence-attestation
description: Tick the contribution licence checkbox on every PR opened for the owner, always, without asking (owner, 2026-10-25)
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The owner instructed on 2026-10-25 that every pull request opened on their
behalf, by the orchestrator or a worker, carries the ticked "Licensing of
contributions" checkbox from the pull request template, so the
contribution-licence guard passes. Never leave it unticked and never ask.

**Why:** the owner is the maintainer and authors these changes; an unticked
box only fails the guard and stalls the merge.

**How to apply:** copy the template's licence line with `- [x]` into every
PR body at creation; fix an existing PR body the moment the guard fails.
