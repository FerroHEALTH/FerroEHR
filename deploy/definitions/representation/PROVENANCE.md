# Representation definition: provenance

One ADL 2 archetype of FerroEHR's default demographic definitions, which the
binary carries and stores at boot when absent, so that the party mapping of
representation (`[authz.party_binding.representation]`, whose default names
it) finds it:

- `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls`: the
  relationship a representative acts through, from the representative's
  `PERSON` to the subject's, named by
  `[authz.party_binding.representation] archetype_id` in its interface form,
  `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1`.

Its name is its type (RM demographic `PARTY_RELATIONSHIP` `Type_validity`),
coded from the value set `ac1`: `at10` (parent) and `at11` (guardian). The
mapping's `relationship_codes` maps each to a code of the openEHR
`subject_relationship` group, `254|parent|` and `264|guardian|`, and boot
refuses a type with no code.

The book's "Default demographic definitions" page lists the whole set with
the configuration that names each file. It is a default: a deployment may
load its own representation archetype, with its own types, and point
`[authz.party_binding.representation]` at it.

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
  RM makes the name the type. Its value set is FerroEHR's own: the two types
  of representative the openEHR `subject_relationship` group names, parent
  and guardian. No CKM archetype or openEHR specification defines a set of
  representation types, so the set is our own modelling.
- The source's `details` are dropped; `time_validity` bounds when the
  representation holds.
- The node ids are ADL 2 id-codes and the terminology is English only.

It was written by hand from the source, never produced by FerroEHR's ADL 1.4
to ADL 2 converter.

## Licence

The CKM source states "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
This adaptation carries the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
