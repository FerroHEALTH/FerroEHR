# `value_alternatives.en.v1`: a repo-authored regression fixture

Derived from this crate's `named_event.en.v1` fixture (itself derived from the
openEHR CNF Robot template `persistent_minimal.opt`, specifications-CNF commit
`33251d2abe5a75c042e11c9385d2e9a79aa15904`, CC-BY-SA 3.0 Unported) and
licensed under the same terms as a derivative work. `LICENSE-CC-BY-SA-3.0` at
the repository root is the licence text; the declaration is in `REUSE.toml`.

## What was changed and why

The carrier issue is #4226: a composition for a template whose `ELEMENT.value`
allows `DV_TEXT` beside a coded `DV_CODED_TEXT` was refused when it carried the
free text. The reported template is not vendored; this fixture reproduces its
shape with synthetic archetypes and adds the other alternative shapes the fix
covers. The single "Reading" element of the skeleton is replaced by four
elements, each with two alternatives under `value`:

- "Nucleic acid" (`at0004`): `DV_TEXT`, then a `DV_CODED_TEXT` over the local
  codes `at0022` (DNA), `at0023` (RNA) and `at0024` (cfDNA), the order and
  codes of the reported template;
- "Amount" (`at0005`): `DV_QUANTITY` or `DV_COUNT`;
- "Recorded" (`at0006`): `DV_DATE` or `DV_DATE_TIME`;
- "Speed" (`at0007`): two `C_DV_QUANTITY` alternatives, `[mi_i]/h` with
  magnitude 0 to 55 and `km/h` with magnitude 0 to 100, after the example in
  ADL 1.4 §"Single-valued Attributes".

The identifiers, the uid and the term definitions are renamed to describe the
fixture. Everything else, including the template envelope, the collapsed
`POINT_EVENT` and the `content` / `HISTORY` / `ITEM_TREE` chain, is the
`named_event.en.v1` skeleton unchanged.
