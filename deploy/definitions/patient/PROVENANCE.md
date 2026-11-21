# Patient PERSON definition: provenance

Three ADL 2 archetypes of FerroEHR's default demographic definitions, which
the binary carries and stores at boot when absent:
`openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0.adls`, named by the default of
`[authz.party_binding.patient]`, and its former versions
`openEHR-DEMOGRAPHIC-PERSON.patient.v2.0.0.adls` and
`openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls`, which stay published
under their ids.

v2.0.0 drops the care relationship node (`id32` to `id35`) of v1.0.0 and
changes nothing else (#6113). A care relationship is a `PARTY_RELATIONSHIP`
of its own at `/demographic/party_relationship`, and FerroEHR refuses one
held in a `PERSON`'s `relationships` (#6055).

v3.0.0 drops the representation slot (`id37`) of v2.0.0 and changes
nothing else (#6170). A representation is a `PARTY_RELATIONSHIP` of its own
(`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v2.0.0`), and
FerroEHR refuses one held in a `PERSON`'s `relationships` (#6079).

Removing a node makes data valid under the former version invalid, a
breaking change, so the major version moves each time (AM Overview,
Semantic Overview: "different major versions are considered different
artefacts"). The rest of this file describes v1.0.0; v2.0.0 is it without
its `id32`, and v3.0.0 without its `id32` and its `id37`.

It is a patient's `PERSON`, the subject of an EHR and the target of a
representation relationship, or the representative who is its source. In `details`: its patient identifiers at
`/details[id2]/items[id3]`, its date of birth at `/details[id2]/items[id4]`,
its sex at `/details[id2]/items[id5]` and the EHR subject references it
stands for at `/details[id2]/items[id36]`. In `identities`: its names, each
named by its usage, with the full, given and family name at
`/identities[id6]/details[id7]/items[id8]`, `id9` and `id10`. In `contacts`:
a postal address (`ADDRESS[id12]`) and an electronic address
(`ADDRESS[id19]`). In `relationships`: a personal relationship (`id23`), a
third-party payer (`id26`) and a healthcare provider (`id29`), the payer and
the provider each with the patient identifiers it issues, and the care
relationships (`id32`) to the teams and units the patient is under the care
of, and a slot (`id37`) for a representation the person holds of another,
such as one of `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1`,
since a patient portal provisions a representative as a `PERSON` of this
archetype (#5611).

## Source

An adaptation of openEHR CKM archetypes, written for FerroEHR. The sources
are the CKM ADL 1.4 exports vendored verbatim under
`corpus/archetypes/ckm/adl14/` by `scripts/vendor/ckm-archetypes.sh`
(provenance in `corpus/archetypes/ckm/PROVENANCE.md`). Every one is a v0
draft that can change without a new id, so `other_details["derived_from"]`
names each with its CKM citeable id and its modification time:

| CKM source | CKM id, modified | What it gives |
|---|---|---|
| `openEHR-DEMOGRAPHIC-PERSON.person-patient.v0` | 1013.1.821, 2019-08-09 | the patient `PERSON` and its three kinds of relationship: to personal contacts, to third-party payers and to providers, each payer and provider with the patient identifiers it issues |
| `openEHR-DEMOGRAPHIC-PERSON.person.v0` | 1013.1.479, 2022-01-20 | the base `PERSON` `person-patient` specialises |
| `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v0` | 1013.1.470, 2018-12-20 | the patient identifiers, as `DV_IDENTIFIER`s |
| `openEHR-DEMOGRAPHIC-CLUSTER.person_birth_data_iso.v0` | 1013.1.491, 2025-09-02 | the birth date |
| `openEHR-DEMOGRAPHIC-CLUSTER.person_additional_data_iso.v0` | 1013.1.503, 2019-08-12 | the sex, coded `at10` to `at13` (male, female, intersex or indeterminate, not declared) |
| `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0` | 1013.1.477, 2019-08-09 | the names, and the name's usage as the name of the `PARTY_IDENTITY`, coded `at20` to `at25` (reporting, newborn, professional or business, maiden, legal, other) |
| `openEHR-DEMOGRAPHIC-ADDRESS.address.v0` | 1013.1.484, 2019-08-08 | the postal address, its type as its name, coded `at30` to `at35` |
| `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0` | 1013.1.486, 2019-08-08 | the electronic address, its type as its name, coded `at40` to `at45` (telephone, fax, mobile, pager, e-mail, URL), and its usage, coded `at50` to `at53` |

The birth date and the sex come from the two CC-BY-SA 3.0 clusters above,
never from `openEHR-DEMOGRAPHIC-ITEM_TREE.person_details.v0`, which holds
the same elements under CC-BY-SA 4.0, so the whole definition stays under
one licence.

## What changed from the sources

- The slots of the CKM PERSON are replaced by the elements they hold: the
  identifier element of `person_identifier`, the birth date of
  `person_birth_data_iso` and the sex of `person_additional_data_iso` in
  `details`; the three names of `person_name` in `identities`; and the
  address lines, postal code, locality, state and country of `address` and
  the usage and details of `electronic_communication` in `contacts`.
- Each coded name keeps the CKM's alternative of free text, as the CKM
  sources allow (`DV_TEXT` or `DV_CODED_TEXT`).
- The payer and provider relationships hold their patient identifiers as
  `DV_IDENTIFIER` elements; the personal relationship's type is a `DV_TEXT`,
  since the source's value set is unbound.
- A record subject element (`id36`) is added, which the CKM source does not
  have: a `DV_IDENTIFIER` named by the coded `record_subject` of the draft
  openEHR access terminology `openehr_access`, holding an `EHR_STATUS` subject reference the
  `PERSON` stands for, such as a pseudonym. An identifier another
  organisation issued belongs in `PARTY.details` (RM Demographic IM §2.1.3).
- A care relationship (`id32`) is added, which the CKM source does not
  have: the patient's relationship to a care team or unit, whose one
  mandatory element is the coded `under_care_of` of the draft openEHR access
  terminology `openehr_access`, with an optional discharge time.
- Every element is optional and every relationship `0..*`, so a patient
  known by one identifier commits `532|complete|`. The CKM source makes the
  details, the name and a contact mandatory.
- The rest of the CKM birth data, the address line structure and the name's
  privacy indicators are dropped.
- The node ids are ADL 2 id-codes and the terminology is English only.

It was written by hand from the sources, never produced by FerroEHR's
ADL 1.4 to ADL 2 converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
This adaptation carries the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
