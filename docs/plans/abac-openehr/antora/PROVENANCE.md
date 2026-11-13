# Provenance: the draft access control model, in the openEHR Antora layout

This tree is the computable part of the draft openEHR change of the ABAC
design (`docs/plans/abac-openehr.md`, tracker issue #5487), laid out as the
access log draft is (`docs/plans/access-log/antora/PROVENANCE.md`), so it
drops into the openEHR RM specification repository beside
`openehr_rm_1.2.0.bmm.json`. It is FerroEHR's own text under BUSL-1.1,
written for submission to openEHR; nothing here has been submitted.

## What is here

| Here | In `specifications-RM` |
|---|---|
| `modules/access_control/` (#5519) | a new module `modules/access_control/` |
| `modules/ROOT/partials/classes/`, `definitions/`, `BMMs/` | new files in the same ROOT directories |
| `tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`, the generator's input (#5506) | `computable/BMM/` beside `openehr_rm_1.2.0.bmm.json` |
| `manifest-entry.json` | one more entry in the `specifications` array of `manifest.json` |
| none | `modules/access_control/nav.adoc` added to the `nav` list of `antora.yml`, and a line in `modules/ROOT/pages/index.adoc` |

The module follows the layout the access log draft records: the
`openEHR/antora-openehr` migration guide and the `ehr_extract` module of
`openEHR/specifications-RM` at the commits named in
`docs/plans/access-log/antora/PROVENANCE.md`. The pages `terminology.adoc`
and `its_rest.adoc` propose changes to the TERM and ITS-REST components;
each says so in a note, and on submission each moves to its own component.
`change_request.adoc` is the text of the problem report and the Change
Request; it moves to the openEHR trackers, not into the RM repository.

Every page carries `partials/draft_notice.adoc`: nothing here has been
submitted to openEHR, and submitting any of it needs the owner's approval.

## Generated parts

The `definitions/`, `BMMs/` and `classes/` partials under
`modules/ROOT/partials/` are rendered from the BMM by
`scripts/render/bmm-partials.sh`, in the table and JSON form of the access
log draft, whose partials the same script reproduces byte for byte. The
`classes/` partials carry the Definition and BMM tabs only; the Effective and
UML tabs, the class SVGs and the package diagram come from bmm-publisher when
the module enters the RM repository, and replace these partials.
`scripts/render/bmm-partials.sh --check` fails when a committed partial
differs from its rendering.

## How the BMM was checked

The repository's BMM loader for draft packages is the draft-module table of
`openehr-codegen` (#5124), on the branch `feat/access-log-draft-types` at
`a599d775b8a1af8af3b5de50b216a9a5aad09b7f`, not yet merged. On 2026-11-12 a
scratch worktree of that commit received this BMM as a second draft row
(module `access_control_draft`, includes `openehr_rm_1.2.0`, resolved against
BASE 1.3.0 and RM 1.2.0); nothing of that run is committed:

- `cargo test -p openehr-codegen --lib drafts`: both drafts compose
  (`BmmSchema::parse_json`, the `includes` check, and the refusal of a type
  that resolves nowhere). A probe that renamed `ACCESS_LABEL` in one property
  to an undefined class was refused with
  `types resolved nowhere ...: ACCESS_DIRECTIVE_SETTINGS -> NO_SUCH_LABEL`.
- `cargo run -p openehr-codegen -- emit-drafts`: the module rendered with
  every class of the package and its canonical-JSON codec, together with the
  RM classes the re-emission closure re-emits because the draft adds
  subtypes of `LOCATABLE`, `VERSIONED_OBJECT` and `ACCESS_CONTROL_SETTINGS`.
- `cargo check -p ferroehr --lib` with the module wired: no error.

The BMM file is written by a script from the class definitions in the design;
the script is not part of the tree, and the file is the source.
