---
name: reactive-console-ui
description: Superseded 2026-10-29 by FerroUI #510: every console draws every page on its server with a little plain JavaScript and no WebAssembly; an in-page change is a module over what the server drew, a navigation one document
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

The owner ruled on 2026-10-29 (FerroUI #510) that every FerroHEALTH console
moves to the VernumBOEK model: server-rendered HTML from FerroUI's Askama
components, a little plain JavaScript, no WebAssembly and no Leptos
hydration. It supersedes the ruling of 2026-10-25, which kept the consoles
reactive with WebAssembly as the normal path (FerroUI #410).

**Why:** the hydrated consoles flickered on every click, lost what the
operator had selected, and redrew the whole page for a tree node or a tab,
because every in-page action was a router navigation that rebuilt its route
and asked the server again. A navigation as one document the browser paints
whole, and an in-page change as a module toggling what the server drew,
keep the state and need no WebAssembly before a click works.

**How to apply:** a console moves whole to FerroUI 0.6, in the order and the
steps of FerroUI `docs/migration-0.6.md` (FerroEHR's viewer is on FerroUI 0.6). A
page is an axum handler that reads its data and returns the document; an
action is a form post; a behaviour the library lacks is a FerroUI issue and
never a script of the console's own. Judge a UI change by the interaction it
gives: an in-page change loads no document and keeps its state, and every
page works with the modules refused.
