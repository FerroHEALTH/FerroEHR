# `any_element_value.en.v1`: a repo-authored regression fixture

Derived from this crate's `named_event.en.v1` fixture (itself derived from the
openEHR CNF Robot template `persistent_minimal.opt`, specifications-CNF commit
`33251d2abe5a75c042e11c9385d2e9a79aa15904`, CC-BY-SA 3.0 Unported) and
licensed under the same terms as a derivative work. `LICENSE-CC-BY-SA-3.0` at
the repository root is the licence text; the declaration is in `REUSE.toml`.

## What was changed and why

The carrier issue is #3933: an `ELEMENT` that leaves its value open admits any
data value, and keeps its node in the web template. The reported template is
not vendored; this fixture reproduces its shape with synthetic archetypes:

- the observation gains a `protocol` `ITEM_TREE` (`at0005`) holding two
  elements. "Test method" (`at0006`) carries no attribute constraint at all,
  the shape of the reported template. "Method detail" (`at0007`) carries a
  `value` attribute with no child constraint, the `value matches {*}` shape.
  "Process state" (`at0008`) constrains its value to `DV_STATE`, the type
  the Simplified Formats give no form (#3934);
- the identifiers, the uid and the term definitions are renamed to describe
  the fixture.

Everything else, including the template envelope and the `content` /
`HISTORY` / `ITEM_TREE` chain with its "Reading" element, is the
`named_event.en.v1` skeleton unchanged.
