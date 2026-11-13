# Care-team GROUP, membership and care relationship definitions: provenance

Four ADL 2 archetypes of FerroEHR's default demographic definitions, which
the binary carries and stores at boot when absent, so that the party mapping
of groups (`[authz.party_binding.group]` and
`[authz.party_binding.membership]`, whose defaults name them) finds them and
SCIM can provision Groups:

1. `openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls`: the `GROUP` of a care
   team, named by `[authz.party_binding.group] template_id`.
2. `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls`: the
   membership a member's `PERSON` holds to the team's `GROUP`, named by
   `[authz.party_binding.membership] archetype_id` in its interface form,
   `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1`.
3. `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1.0.0.adls`: an example
   care-team membership a professional's `PERSON` holds to the team's
   `GROUP`, whose details carry `member_of` of FerroEHR's terminology
   `ferroehr-operations`.
4. `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v1.0.0.adls`: an
   example care relationship a patient's `PERSON` holds to a care team's
   `GROUP` or a unit, with the admission as its `time_validity`, whose details
   carry `under_care_of` of that terminology.

FerroEHR reads a care relationship by its code in any archetype, never by
the id of the third or fourth file; a deployment may model one in an
archetype of its own. The care delivery environment writes them under the
`write_care_relationship` grant.

The book's "Default demographic definitions" page lists the whole set with
the configuration that names each file. They are defaults: a deployment may
load its own and point `[authz.party_binding]` at them.

## Source

openEHR CKM publishes no `GROUP` archetype and no `PARTY_RELATIONSHIP`
archetype. Both are adaptations of the CKM archetypes their
`other_details["derived_from"]` names, written for FerroEHR. The sources are
the CKM ADL 1.4 exports vendored verbatim under `corpus/archetypes/ckm/adl14/`
by `scripts/vendor/ckm-archetypes.sh` (provenance in
`corpus/archetypes/ckm/PROVENANCE.md`). Every source is a v0 draft that can change without a new id, so
`other_details["derived_from"]` names each with its CKM citeable id and
its modification time.

| Definition | CKM sources (CKM id, modified) |
|---|---|
| GROUP | `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` (1013.1.475, 2019-08-09), `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.organisation_name.v0` (1013.1.476, 2019-08-09) |
| Membership | `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` (1013.1.475, 2019-08-09; its inline `PARTY_RELATIONSHIP`) |
| Member of | `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` (1013.1.475, 2019-08-09; its inline `PARTY_RELATIONSHIP`) |
| Under care of | `openEHR-DEMOGRAPHIC-PERSON.person-patient.v0` (1013.1.821, 2019-08-09; its inline `PARTY_RELATIONSHIP`) |

## What changed from the sources

- The GROUP takes the shape of the CKM ORGANISATION, since both classes
  inherit `ACTOR` (RM `org.openehr.rm.demographic.group.adoc`,
  `org.openehr.rm.demographic.organisation.adoc`):
  identifiers in `details` and the name in `identities`. The identifier slot
  of the source is replaced by a repeated `DV_IDENTIFIER` element, which holds
  a provisioned group's `externalId`, and the name identity of
  `organisation_name` is reduced to its name element, which holds the
  `displayName`.
- The contacts and the relationships of the CKM ORGANISATION are dropped: a
  team's members hold their memberships themselves.
- The membership is the `PARTY_RELATIONSHIP` the CKM ORGANISATION constrains
  inline, as an archetype of its own, with its source the member's `PERSON`
  and its target the team's `GROUP` (RM demographic §Party Relationships).
  The type of relationship element becomes the member's optional role in the
  team; the type of the relationship is its name (`PARTY_RELATIONSHIP`
  `Type_validity`).
- The two care relationships keep that shape and hold, as their one
  mandatory element, the coded care relationship code, after the
  representation definition; `member_of` keeps the optional role in the team,
  and `under_care_of` replaces it with an optional discharge time.
- Every element is optional, so a team provisioned with only a name commits
  `532|complete|`.
- The node ids are ADL 2 id-codes and the terminology is English only.

They were written by hand from the sources, never produced by FerroEHR's
ADL 1.4 to ADL 2 converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
These adaptations carry the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
