# Professional PERSON and ROLE definitions: provenance

Two ADL 2 archetypes a deployment loads with `ferroehr definitions load`
before `serve`, so that its party mapping (`[authz.party_binding.person]` and
`[authz.party_binding.professional_role]`) finds its templates at boot and
SCIM can provision its professionals. The development lanes, the conformance
harness, the viewer browser lane, the compose quickstart and the Helm CI
values load them in this order:

1. `openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls`
2. `openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls`

They are a test and quickstart set. A production deployment loads the
templates its own modelling chose.

## Source

Both are adaptations of openEHR CKM archetypes, written for FerroEHR. Each
names its sources in `other_details["derived_from"]`; the sources are the
CKM ADL 1.4 exports vendored verbatim under `corpus/archetypes/ckm/adl14/` by
`scripts/vendor/ckm-archetypes.sh` (provenance in
`corpus/archetypes/ckm/PROVENANCE.md`):

| Fixture | CKM sources |
|---|---|
| PERSON | `openEHR-DEMOGRAPHIC-PERSON.person.v0`, `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v0`, `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0`, `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0` |
| ROLE | `openEHR-DEMOGRAPHIC-ROLE.individual_provider.v0`, `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0` |

## What changed from the sources

- The slots of the CKM PERSON (`person_details`, `person_name`, `address`,
  `electronic_communication`) are replaced by the elements the mapping
  writes: the identifier element of `person_identifier` in `details`, the
  full, given and family name of `person_name` in `identities`, and the
  email element of `electronic_communication` in `contacts`.
- Every element is optional, so a person provisioned with only a login
  identifier commits `532|complete|`. The CKM sources make several of them
  mandatory.
- The CKM ROLE keeps its name only; capabilities, contacts and
  relationships are dropped.
- The node ids are ADL 2 id-codes and the terminology is English only.

They were written by hand from the sources, never produced by FerroEHR's
ADL 1.4 to ADL 2 converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
These adaptations carry the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
