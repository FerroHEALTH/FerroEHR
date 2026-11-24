---
name: mermaid-diagrams
description: every diagram in every Ferro product is a mermaid block, never ASCII art; render each fence with mermaid-cli before a pull request (owner rule 2026-11-24, every family repo)
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Owner, 2026-11-24: every diagram in every Ferro product is Mermaid, never
ASCII art. That holds for the book pages, the READMEs, `docs/` and the
`.claude` pages: a `sequenceDiagram` for a flow, a `flowchart` for
components and deployments, a `stateDiagram-v2` for states. Terminal
output, log transcripts and directory trees are text, not diagrams, and
stay in a `text` fence.

**Why:** the owner's words: "ASCII diagrams aren't readable". GitHub renders
a mermaid fence, and every family mdBook renders it from a mermaid script
the book serves itself, so the page's Content-Security-Policy holds.

**How to apply:** draw a new diagram as a ```` ```mermaid ```` fence, and
replace an ASCII diagram whenever you touch its page. Render every fence
before the pull request with
`npx -y -p @mermaid-js/mermaid-cli mmdc -p <puppeteer.json> -i x.mmd -o x.svg`,
the puppeteer config's `executablePath` pointing at the installed Google
Chrome, and build the book. Use `<br/>` for a line break in a label, never
`\n`; keep `;` out of sequence messages; quote a label that carries
parentheses, colons or braces. Related: [[family-memories-every-repo]].
