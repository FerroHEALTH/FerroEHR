---
paths: ["crates/**/*.rs", "app/**/*.rs", "tools/**/*.rs"]
---

# Comments & documentation (RFC 505 + RFC 1574)

The authority is the official Rust API documentation conventions — RFC 505
(https://rust-lang.github.io/rfcs/0505-api-comment-conventions.html) and
RFC 1574 (https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html)
— plus the rustdoc book. Owner directive 2026-08-04: comments are the
fastest-rotting artifact in the repo; essays in code are banned. This file is
the single home of the comment rules; `rust-style.md` points here.

## The prime rule: a comment earns its lines

Code says WHAT; a comment exists only for what the code cannot show — the
spec citation, the non-obvious why, the constraint. Everything else already
has a durable home and goes there, not into the source:

| Content | Home |
|---|---|
| Adjudications, spec-conflict essays, history | the PR description / tracker issue |
| Design decisions | `docs/architecture.md`, the crate `CLAUDE.md` |
| API usage, contracts, examples | doc comments (`///`) |
| What changed and why it is correct | the PR — never the code |

**Comments are lean and current** (owner hard rule, 2026-11-23). Write a
comment only when the code cannot say it: a spec citation, a non-obvious
why, a constraint. Most items need one summary line or none. A comment never
describes the old way, past figures or how the code got here: no
"previously…", "now correctly…", "where it sent 41", "once X moved, it fell
by its drop", no measurement trail. A budget or ceiling constant gets at
most one line naming its issue. When code changes, its comment is rewritten
whole to describe only the new code; another "then … now …" sentence is
never appended. History belongs in the PR and in git.

## Budgets (machine-enforced — `scripts/checks/comment-style.sh`)

- `// NOTE:` = a citation + ONE sentence, **max 3 physical lines**. The
  full adjudication lives on the issue/PR.
- A plain `//` comment run is **max 8 physical lines**. Longer prose is
  either API documentation (move it into the item's `///` docs) or a record
  (move it to the PR/issue).
- An item's `///` doc is **max 12 prose lines**, counting neither its
  `# Errors`, `# Panics`, `# Examples` and `# Safety` sections, its code
  blocks nor its table rows. The cap is the longest item doc the #6368
  sweep kept: 81 docs sit at 12 prose lines, 90 at 11, 134 at 10. A doc
  the served OpenAPI document or the configuration schema reads (`utoipa`,
  `ToSchema`, `IntoParams`, `OpenApi` or `JsonSchema` on the item) is
  product text and exempt.
- A phrase that dates the code ("has since", "fell by", "measured beside the
  merge base", "where it sent N", "since #NNNN", "before #NNNN", "formerly")
  is refused in any comment.
- Block comments (`/* … */`, `/** … */`) are banned — "Avoid block comments.
  Use line comments instead" (RFC 505). `/* arg */` parameter labels become
  named locals.

## Doc comments (`///`)

- `///` documents items; `//!` is ONLY for crate- and module-level docs
  ("nothing else" — RFC 1574); above a `mod` block prefer `///` outside it.
- **Summary line**: the first line is a single short sentence, third person
  singular present indicative ("Returns…", "Creates…" — never "Return" or
  "This function returns"), properly punctuated, followed by a blank `///`
  line before any detail (`clippy::too_long_first_doc_paragraph`).
- Full sentences over fragments; American English spelling.
- Markdown with `#` top-level section headings, in this order where present:
  `# Examples` (always plural, even for one example), `# Panics`,
  `# Errors`, `# Safety` (`missing_errors_doc` / `missing_panics_doc`
  enforce presence; `unnecessary_safety_doc` bans `# Safety` on safe code).
- Code fragments in backticks (`clippy::doc_markdown`); longer examples in
  triple-backtick blocks, non-Rust blocks explicitly tagged (```text) —
  rustdoc tests every untagged block as Rust. Doctest shapes: testing.md
  §Test shapes.
- Name generic types fully (`Option<T>`, not `Option`); link with intra-doc
  links (`[`Type`]`) and reference-style links; bare URLs in `<…>`.
- Doc comments state the CURRENT contract only — no history, no
  adjudication trail. Citations follow spec-adherence.md (vendored specs +
  official external docs only).
- Generated crates get their docs FROM THE EMITTER — a doc defect in a
  `// @generated` file is an `openehr-codegen` fix + regeneration.

## Annotation vocabulary (the only sanctioned markers)

- `// TODO(#NNNN): <what is missing>` — pending work, ALWAYS with its
  tracker issue (owner 2026-08-01). Deferred work is a TODO, never prose
  ("lands later", "deferred to X" is banned); phase/plan markers
  (A5/P16/W-nn) are banned tracker IDs.
- `// NOTE: <citation + one sentence>` — a SETTLED decision: the spec
  citation, or the explicit flag "no openEHR spec governs this — our own
  design/extension". If it describes something not yet done, it is a TODO.
- `// SAFETY:` — reserved for `unsafe` (forbidden anyway;
  `unnecessary_safety_comment` denies misuse).
- No other marker form exists: `FIXME`, `HACK`, `XXX`, `WIP`, and any
  bespoke vocabulary fail the guard.

## Enforcement register

- `scripts/checks/comment-style.sh` — block comments, TODO(#N) form, banned
  marker vocabulary, NOTE ≤ 3 lines (≤ 8 in doc comments), `//` runs ≤ 8
  lines, punctuation-only comment lines (sweep residue), backtick-quoted
  markers used AS markers on doc lines (leading/bullet/parenthetical — a
  mid-sentence description stays legal), and empty `# Errors`/`# Panics`
  doc sections (#2981), the narration phrases and the item doc cap (#6368).
  Runs per-edit (the `rust_fmt_clippy.sh` PostToolUse
  hook) and on PRs (the `comment-style` CI job, running `--all` over the
  whole tree — the legacy sweep (#1870) is done).
- `clippy::too_long_first_doc_paragraph` (nursery cherry-pick, CI
  `-D warnings`) — the RFC 1574 summary line.
- Already-active doc lints: `doc_markdown`, `missing_errors_doc`,
  `missing_panics_doc` (pedantic = deny), `unnecessary_safety_comment`,
  `unnecessary_safety_doc`, the `[workspace.lints.rustdoc]` table + CI doc
  job.
- Review-enforced (no tool can judge them): change-narration the phrase
  list misses, a comment the code already says, prose
  deferrals, third-person summary phrasing, essays relocated into doc
  comments to dodge the `//` budget, and module-doc (`//!`) block LENGTH —
  adjudicated on #2981: the tree's longest module docs are governing-section
  maps and matching-rule contracts, longer than any essay the #2942 sweep
  condensed, so a numeric cap either catches nothing or forces condensing
  legitimate reference docs; essay-vs-reference is judgment.
