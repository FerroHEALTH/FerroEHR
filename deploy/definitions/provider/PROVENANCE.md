# Healthcare provider ORGANISATION definition: provenance

One ADL 2 archetype of FerroEHR's default demographic definitions, which a
deployment loads with `ferroehr definitions load` before `serve`:

- `openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls`: a
  healthcare provider's `ORGANISATION`. It is the provider a system acts for,
  the target of the acts-for relationship, and a federation partner, the
  performer of a federation partner's `ROLE` (the book's "Operational
  policy" page). Its provider identifiers are at `/details[id2]/items[id3]`,
  and `details` are mandatory, since the identifier names the provider (RM
  demographic §Party Identification). Its names are in `identities`, each
  named by its usage, with the name at `/identities[id4]/details[id5]/items[id6]`,
  and its postal (`ADDRESS[id8]`) and electronic (`ADDRESS[id15]`)
  addresses in `contacts`.

No configuration key names it.

## Source

An adaptation of openEHR CKM archetypes, written for FerroEHR. The sources
are the CKM ADL 1.4 exports vendored verbatim under
`corpus/archetypes/ckm/adl14/` by `scripts/vendor/ckm-archetypes.sh`
(provenance in `corpus/archetypes/ckm/PROVENANCE.md`). Each is a v0 draft,
so `other_details["derived_from"]` names it with its CKM citeable id and its
modification time:

| CKM source | CKM id, modified | What it gives |
|---|---|---|
| `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` | 1013.1.475, 2019-08-09 | the `ORGANISATION` with its identifiers, name and contacts |
| `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.organisation_name.v0` | 1013.1.476, 2019-08-09 | the name, and its usage as the name of the `PARTY_IDENTITY`, coded `at10` to `at17` |
| `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier-provider.v0` | 1013.1.833, 2019-08-12 | the provider identifier, as a `DV_IDENTIFIER` |
| `openEHR-DEMOGRAPHIC-ADDRESS.address-provider.v0` | 1013.1.499, 2019-08-08 | the postal address, its type coded `at20` to `at25` |
| `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication-provider.v0` | 1013.1.487, 2019-08-08 | the electronic address, its type coded `at30` to `at35` and its usage `at40` to `at43` |

## What changed from the sources

- The slots of the CKM ORGANISATION are replaced by the elements they hold:
  the identifier of `person_identifier-provider` in `details`, the name of
  `organisation_name` in `identities`, and the address lines, postal code,
  locality, state and country of `address-provider` and the usage and
  details of `electronic_communication-provider` in `contacts`.
- `details` and one provider identifier are mandatory; every other element
  is optional.
- Each coded name keeps the CKM's alternative of free text.
- The CKM ORGANISATION's inline relationship is dropped: the acts-for
  relationship is held by the system's `AGENT`, its source.
- The communication privacy and the address line structure of the provider
  addresses are dropped.
- The node ids are ADL 2 id-codes and the terminology is English only.

It was written by hand from the sources, never produced by FerroEHR's
ADL 1.4 to ADL 2 converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
This adaptation carries the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
