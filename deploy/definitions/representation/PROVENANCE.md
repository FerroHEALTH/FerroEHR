# Representation definition: provenance

Two ADL 2 archetypes of FerroEHR's default demographic definitions, which
the binary carries and stores at boot when absent, as examples of a
representation:

- `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v2.0.0.adls`: a
  representation of its own, written at `/demographic/party_relationship`,
  from the representative's `PERSON` to the subject's, in its interface form
  `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v2`. Its name is
  `represents` of the draft openEHR access terminology `openehr_access`, and
  its details hold the elements of the `representation element` group:
  `basis` (`id3`, 1..1), `provider` (`id5`, 1..1) and `evidence` (`id7`,
  0..1), with an optional code of the openEHR `subject_relationship` group
  (`id9`), such as `254|parent|`, which the committer and the access log
  name (#6079).
- `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls`: the
  former example, held in a `PERSON`'s relationships or of its own, whose
  name is its type from the value set `ac1` and whose details carry the
  representation code (`id4`) and a `subject_relationship` code (`id6`).
  FerroEHR refuses that form, which has no `basis` and no `provider`; the
  file stays published under its id.

FerroEHR recognises a representation by the codes its `name` and its
elements' names carry, in any archetype, never by these archetypes' ids.
The book's "Default demographic definitions" page lists the whole set. Each
is an example: a deployment may model a representation in an archetype of
its own, and FerroEHR honours it by the codes.

## Source

openEHR CKM publishes no `PARTY_RELATIONSHIP` archetype. This one adapts the
person-to-person `PARTY_RELATIONSHIP` the CKM `PERSON` archetypes constrain
inline, as its `other_details["derived_from"]` names: the "personal
relationships" of `openEHR-DEMOGRAPHIC-PERSON.person.v0` (CKM 1013.1.479,
modified 2022-01-20, node `at0004`) and the "Personal relationship" of
`openEHR-DEMOGRAPHIC-PERSON.person-patient.v0` (CKM 1013.1.821, modified
2019-08-09, node `at0004.1`). Every source is a v0 draft that can change without a new id, so
`other_details["derived_from"]` names each with its CKM citeable id and
its modification time. The sources are the CKM ADL 1.4
exports vendored verbatim under `corpus/archetypes/ckm/adl14/` by
`scripts/vendor/ckm-archetypes.sh` (provenance in
`corpus/archetypes/ckm/PROVENANCE.md`).

## What changed from the source

- The relationship is an archetype of its own, held in the representative's
  `PERSON.relationships` (RM demographic §Party Relationships).
- The source's "Type of relationship" element, a text or a code from an
  unbound value set, becomes the coded `name` of the relationship, since the
  RM makes the name the type. Its value set is FerroEHR's own: parent,
  guardian and authorised proxy. No CKM archetype or openEHR specification
  defines a set of representation types, so the set is our own modelling.
- The source's `details` are replaced by two coded elements: the
  representation code of `openehr_access` and the openEHR
  `subject_relationship` code. `time_validity` bounds when the representation
  holds.
- The node ids are ADL 2 id-codes and the terminology is English only.

It was written by hand from the source, never produced by FerroEHR's ADL 1.4
to ADL 2 converter.

## Licence

The CKM source states "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
This adaptation carries the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
