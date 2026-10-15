# Vendored openEHR BMM meta-model (ITS-BMM), verbatim

Source: https://github.com/openEHR/specifications-ITS-BMM
Pinned commit: `37b317396eb9b5f03c4c422296a15bdd665028dd` (master)
Fetched: 2026-07-04 · Layout: **verbatim upstream `components/`** mirror,
`json/` and `odin/` serializations (the `yaml/` one is not vendored: nothing
here reads it).
Reproduced by: `scripts/vendor/codegen-inputs.sh` (`--check` proves the
committed tree, overlays included, matches the pins below byte for byte).
License: Apache-2.0 (the ITS-BMM repo's `LICENSE`; root reference copy
`LICENSE-APACHE-2.0`). The three component-repo overlay files below come from
`specifications-BASE`, `specifications-RM` and `specifications-TERM`, whose
repos are CC-BY-SA 3.0 — each is the same BMM artifact ITS-BMM redistributes
under Apache-2.0, taken from the component repo only until ITS-BMM
republishes it.

## Component-repo overlays (json only)

ITS-BMM had not republished these versions at our ITS-BMM pin, so the json
file is vendored verbatim from the component repo ITS-BMM redistributes from.
Each is byte-identical to its reference twin under
`docs/specs/openehr/<C>/computable/BMM/`, and `codegen-inputs.sh` fails when a
twin is missing or differs. The component repos publish json only, so the
ITS-BMM `odin/` serialization of each of these three versions predates the
overlay; `codegen-inputs.sh` drops it rather than vendor a second, different
text of the release (see "The odin serialization" below). Fold each back to a plain ITS-BMM pin at
the next ITS-BMM bump.

| File | Component repo | Commit | Adopted | Why |
|---|---|---|---|---|
| `components/BASE/json/openehr_base_1.3.0.bmm.json` | specifications-BASE | `e48795762a0648cbe5701be58d42ec5df0c701a7` | 2026-07-25 | SPECBASE-48 invariants, SPECAM-82 CODE_PHRASE package move, SPECPR-426/386/460 fixes |
| `components/RM/json/openehr_rm_1.2.0.bmm.json` | specifications-RM | `66d3ac45587e4532a94d5fd27ca24bcf049f5bf3` | 2026-07-26 | asterisk HTML entities normalized to a literal `*` in aliases (upstream PR #33); zero generated-output change |
| `components/TERM/json/openehr_term_3.1.0.bmm.json` | specifications-TERM | `1232e59bb679928b921b305b6eca4d0b5e1e03f0` | 2026-10-14 | `status` default facet `""` -> `"active"` on CODE_SET, TERMINOLOGY_GROUP, CODE, TERMINOLOGY_CONCEPT; `includes` names `openehr_base_1.3.0` |

One knock-on of the BASE overlay: ITS-BMM had independently fixed a doc typo
(`senstive`->`sensitive`) in `RESOURCE_DESCRIPTION_ITEM.other_details` (see
the 2026-07-04 note below); the specifications-BASE file still carries the
typo, so the generated doc comment regresses to authentic component-repo text.

The ITS-BMM `components/` tree is vendored under `components/` here — all
released versions in two serializations (**json**: 18 files; **odin**: 15, the
three overlaid versions dropped).
The `json` files are upstream's source of truth (`odin`/`yaml` are generated
from them by `bmm-publisher`); we consume **json** for codegen. Upstream's
`yaml/` serialization is excluded by `codegen-inputs.sh` because no code,
test or script reads it.

## Codegen input (the pinned versions `openehr-codegen` loads)

`openehr-codegen`'s `main.rs` loads these JSON files (paths relative to this
dir), per `docs/VERSIONS.md`:

| Const | Path |
|---|---|
| BASE | `components/BASE/json/openehr_base_1.3.0.bmm.json` |
| RM   | `components/RM/json/openehr_rm_1.2.0.bmm.json` |
| TERM | `components/TERM/json/openehr_term_3.1.0.bmm.json` |
| AM 1.4 | `components/AM/json/openehr_am_1.4.0.bmm.json` |
| AM 2.4 | `components/AM/json/openehr_am_2.4.0.bmm.json` |
| LANG | `components/LANG/json/openehr_lang_1.1.0.bmm.json` |
| LANG (BMM-3) | `components/LANG/json/openehr_lang_1.1.0-bmm3.bmm.json` |

Why LANG spans two files: the primary one carries the persisted-BMM (`P_BMM_*`),
the `EXPR_*` model, and `STATEMENT_SET`/`ASSERTION` (referenced by AM's
`rules`/`includes`); the `-bmm3` file carries the full `BMM_*` object model and
the `EL_*` expression language (referenced by AM's persisted-archetype `rules`).
Neither alone resolves every AM reference, so both are merged
(`BmmSchema::combined`).

## Everything else vendored (available, not currently codegen input)

All other versions are present for completeness / future spec bumps: RM
1.0.2/1.0.3/1.0.4/1.1.0; BASE 1.0.4/1.1.0/1.2.0; AM 2.2.0/2.3.0; LANG 1.0.0;
TERM 3.0.0, plus the `odin/` serialization of every version not overlaid.

The json files are the deterministic input to `openehr-codegen`.

## The odin serialization (reference, read by `openehr-lang` tests)

The `odin/` files are not codegen input. `openehr-lang`'s
`tests/it/bmm_json_reader.rs` reads every json/odin pair of one release through
both P_BMM readers and requires the same `P_BMM_SCHEMA` value. The other
`openehr-lang` tests over the component schemas (`tests/it/vendor_bmm_schema.rs`,
`tests/it/el_assertions.rs`) read the json files codegen reads. For BASE 1.3.0,
RM 1.2.0 and TERM 3.1.0 no ODIN file is vendored: the ITS-BMM copies at
`37b31739…` predate the json overlays above, and `codegen-inputs.sh` drops them.

Note (drift caught 2026-07-04): re-vendoring verbatim at this commit corrected a
one-character doc typo (`senstive`->`sensitive`) in BASE's
`RESOURCE_DESCRIPTION_ITEM.other_details`, so the generated `openehr-base` doc
comment now matches authentic upstream.
