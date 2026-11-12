# Professional PERSON and ROLE definitions: provenance

Two ADL 2 archetypes of FerroEHR's default demographic definitions, which
the binary carries and stores at boot when absent, first and in this order,
so that the party mapping (`[authz.party_binding.person]` and
`[authz.party_binding.professional_role]`, whose defaults name them) finds
its templates and SCIM can provision its professionals:

1. `openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls`
2. `openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls`

The book's "Default demographic definitions" page lists the whole set with
the configuration that names each file. They are defaults: a deployment may
load its own templates and point `[authz.party_binding]` at them.

## Source

Both are adaptations of openEHR CKM archetypes, written for FerroEHR. Each
names its sources in `other_details["derived_from"]`; the sources are the
CKM ADL 1.4 exports vendored verbatim under `corpus/archetypes/ckm/adl14/` by
`scripts/vendor/ckm-archetypes.sh` (provenance in
`corpus/archetypes/ckm/PROVENANCE.md`). Every source is a v0 draft that can change without a new id, so
`other_details["derived_from"]` names each with its CKM citeable id and
its modification time.

| Fixture | CKM sources (CKM id, modified) |
|---|---|
| PERSON | `openEHR-DEMOGRAPHIC-PERSON.person.v0` (1013.1.479, 2022-01-20), `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v0` (1013.1.470, 2018-12-20), `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0` (1013.1.477, 2019-08-09), `openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0` (1013.1.486, 2019-08-08) |
| ROLE | `openEHR-DEMOGRAPHIC-ROLE.individual_provider.v0` (1013.1.482, 2019-08-09), `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0` (1013.1.477, 2019-08-09), `openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0` (1013.1.824, 2019-08-08) |

## What changed from the sources

- The slots of the CKM PERSON (`person_details`, `person_name`, `address`,
  `electronic_communication`) are replaced by the elements the mapping
  writes: the identifier element of `person_identifier` in `details`, the
  full, given and family name of `person_name` in `identities`, and the
  email element of `electronic_communication` in `contacts`.
- Every element is optional, so a person provisioned with only a login
  identifier commits `532|complete|`. The CKM sources make several of them
  mandatory.
- The PERSON's name is named by its usage, as `person_name` codes it
  (`at20` to `at25`: reporting, newborn, professional or business, maiden,
  legal, other), and its email address by its type, as
  `electronic_communication` codes it (`at30` to `at35`), with its usage
  (`at40` to `at43`). Each keeps the CKM's alternative of free text, which is
  what SCIM provisioning writes.
- The CKM ROLE's employer/employee relationship is dropped: employment is
  application data, which the CDR does not enforce.
- The CKM ROLE keeps its name and its `CAPABILITY` slot, filled inline with
  the CKM `individual_credentials` archetype: the field of practice, whether
  it is the primary one, each registration (its id, registering body,
  status, period, state and country) and each qualification. The
  registration status keeps the CKM's eight codes as `at10` to `at17`; the
  CKM's unbound value sets for the field of practice, the qualification
  level, the state and the country become `DV_TEXT`. Every element the CKM
  makes mandatory at the top of the credentials is optional, and the
  capability is `0..*`, so a ROLE without credentials commits
  `532|complete|`. Contacts and relationships are dropped.
- The node ids are ADL 2 id-codes and the terminology is English only.

They were written by hand from the sources, never produced by FerroEHR's
ADL 1.4 to ADL 2 converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
These adaptations carry the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
