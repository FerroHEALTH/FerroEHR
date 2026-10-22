# `slot_fill_twice.en.v1`: a repo-authored regression fixture

Derived from this crate's `named_event.en.v1` fixture (itself derived from the
openEHR CNF Robot template `persistent_minimal.opt`, specifications-CNF commit
`33251d2abe5a75c042e11c9385d2e9a79aa15904`, CC-BY-SA 3.0 Unported) and
licensed under the same terms as a derivative work. `LICENSE-CC-BY-SA-3.0` at
the repository root is the licence text; the declaration is in `REUSE.toml`.

## What was changed and why

The carrier issue is #3920: a template may fill the same archetype at two
places, each fill with its own root term, and each fill's web-template node id
comes from its own term. The reported template is not vendored; this fixture
reproduces its shape with synthetic archetypes:

- the event's name constraint is removed, so the single event is collapsed
  without a name;
- the `ITEM_TREE` element is replaced by two `CLUSTER` slot fills of
  `openEHR-EHR-CLUSTER.slot_filler.v1`. The first sits directly in the tree,
  is renamed "Genome assembly" by its root term and a `name/value`
  constraint, and comes first in document order. The second sits inside an
  `openEHR-EHR-CLUSTER.variant.v1` fill ("Substitution variant") and keeps the
  root term "Reference sequence";
- the identifiers, the uid and the term definitions are renamed to describe
  the fixture.

Everything else, including the template envelope and the `content` /
`HISTORY` / `ITEM_TREE` chain, is the `named_event.en.v1` skeleton unchanged.
