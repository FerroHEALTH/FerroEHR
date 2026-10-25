---
name: reactive-console-ui
description: The consoles must be reactive with no full page reload for an interaction (owner, 2026-10-25); WebAssembly is the normal path and the no-WebAssembly page only the fallback
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The owner ruled on 2026-10-25 that every FerroHEALTH console stays reactive:
a link, a form or an action never reloads the whole document. WebAssembly is
the normal path; a page that works without it is the fallback, never the
design target.

**Why:** FerroUI #196 (PR #223, 2026-10-22) moved the consoles to islands
mode on bundle size and time-to-interactive alone and made every action a
plain form post, so each click became a full page load, which the owner
found slow and unacceptable. FerroUI #410 restores client navigation.

**How to apply:** user experience outranks bundle bytes. Judge a UI change
by the interaction it gives, measure navigations as well as first load, and
never land a design whose every interaction reloads the page.
