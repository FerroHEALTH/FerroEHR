# Default demographic definitions

Party binding and SCIM provisioning write demographic parties under the
templates and archetypes the [party mapping](config-auth.md#the-party-mapping)
names, and the server refuses to boot while the definition store lacks one of
them. FerroEHR ships a default set of seven ADL 2 archetypes that the
documented mappings name, so a deployment can load them and boot those
mappings as they stand:

- a professional's `PERSON` and the professional `ROLE` it performs, with
  the professional's registrations and qualifications as its capabilities;
- a care-team `GROUP`, which a SCIM Group is provisioned as;
- the membership `PARTY_RELATIONSHIP` a member's `PERSON` holds to its
  `GROUP`;
- the representation `PARTY_RELATIONSHIP` a representative acts through;
- a patient's `PERSON`, the subject a representation targets;
- a healthcare provider's `ORGANISATION`, the provider a system acts for and
  a federation partner.

The policy definitions of [Operational policy](../operational-policy.md#the-policy-parties) are
not in this set: the binary carries their defaults and stores each one at
boot when no definition of its id is stored.

These are defaults. A deployment may load its own templates instead, or
beside them, and point `[authz.party_binding]` at those. Nothing in FerroEHR
depends on these ids beyond the configuration that names them.

## The files

Each file is in the repository under `deploy/definitions/` and is attached to
every release as an asset of its own, with a `.sha256sum` file beside it.

| File | What it is for | The key that names it |
|---|---|---|
| [`openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls) | A professional's `PERSON`: the binding key, `userName` and `externalId` as identifiers in `details` (`/details[id2]/items[id3]`), the full, given and family name in `identities` (`/identities[id4]/details[id5]/items[id6]`, `id7`, `id8`), and a work email in `contacts` (`/contacts[id9]/addresses[id10]/details[id11]/items[id12]`). | `[[authz.party_binding.person.template]] template_id` and the `path` keys of `[[authz.party_binding.person.attribute]]` |
| [`openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls) | The professional `ROLE` that makes a `PERSON` a professional while it is valid, with a `CAPABILITY` per credential: the field of practice and each registration and qualification, as the CKM `individual_credentials` archetype records them (`/capabilities[id6]/credentials[id7]/items[id10]` is a registration). | `[authz.party_binding.professional_role] template_id` |
| [`openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls) | A care team's `GROUP`: its `externalId` as an identifier in `details` (`/details[id2]/items[id3]`) and its `displayName` in `identities` (`/identities[id4]/details[id5]/items[id6]`). | `[authz.party_binding.group] template_id` |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls) | The membership of a `PERSON` in a `GROUP`, held in the member's `relationships`, with an optional role in the team. | `[authz.party_binding.membership] archetype_id`, as `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1` |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls) | The relationship from a representative's `PERSON` to the subject's, whose coded name is its type: `at10` (parent) or `at11` (guardian). | `[authz.party_binding.representation] archetype_id`, as `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1`, and its `relationship_codes` |
| [`openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls) | A patient's `PERSON`: its patient identifiers (`/details[id2]/items[id3]`), date of birth (`id4`) and sex (`id5`) in `details`, its names in `identities`, each named by its usage (`/identities[id6]/details[id7]/items[id8]`, `id9`, `id10`), a postal and an electronic address, and its relationships to personal contacts, third-party payers and providers, each payer and provider with the patient identifiers it issues. | none: the party mapping maps professionals |
| [`openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls) | A healthcare provider's `ORGANISATION`: its provider identifiers in `details` (`/details[id2]/items[id3]`, such as the Dutch URA, mandatory), its names named by their usage, and a postal and an electronic address. The acts-for relationship targets it, and a federation partner is one. | none |

The links above fetch the newest release's copy. To fetch the copy of a
given release, replace `vX.Y.Z` with its tag:

```sh
base=https://github.com/FerroHEALTH/FerroEHR/releases/download/vX.Y.Z
for file in \
  openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls; do
  curl -fsSLO "$base/$file"
  curl -fsSLO "$base/$file.sha256sum"
  sha256sum --check "$file.sha256sum"
done
```

The same files at a release tag are in the repository at
`https://github.com/FerroHEALTH/FerroEHR/tree/vX.Y.Z/deploy/definitions`:

- [`deploy/definitions/professional/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/deploy/definitions/professional):
  the `PERSON` and the `ROLE`;
- [`deploy/definitions/care-team/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/deploy/definitions/care-team):
  the `GROUP` and the membership;
- [`deploy/definitions/representation/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/deploy/definitions/representation):
  the representation;
- [`deploy/definitions/patient/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/deploy/definitions/patient):
  the patient `PERSON`;
- [`deploy/definitions/provider/`](https://github.com/FerroHEALTH/FerroEHR/tree/main/deploy/definitions/provider):
  the provider `ORGANISATION`.

The links above point at `main`; replace `main` with a tag for that
release's copy. Each directory's `PROVENANCE.md` names the openEHR CKM
archetypes the files adapt and every change from them. openEHR CKM publishes
no `GROUP` and no `PARTY_RELATIONSHIP` archetype, so the `GROUP` and the
membership adapt the CKM `ORGANISATION` archetype and its inline
relationship, the representation adapts the person-to-person relationship of
the CKM `PERSON` archetypes, and the representation types are FerroEHR's own
modelling. Every CKM source is a v0 draft, so each file names it with its
CKM id and modification time. The `CAPABILITY` of the professional `ROLE` is
CKM's `individual_credentials`: the openEHR RM and CKM use a capability for a
performer's qualifications. The files carry the
CKM sources' licence, Creative Commons Attribution-ShareAlike 3.0.

## Loading them

Load them, in the order of the table, before the server starts:

```sh
ferroehr definitions load \
  openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls
```

A file stored already with the same text is left alone, so the step runs at
every start ([`ferroehr definitions load`](config-cli.md#ferroehr-definitions-load)).
The shipped deployments load all seven:

- the [Docker Compose quickstart](compose.md), through its
  `ferroehr-definitions` one-shot, which the Keycloak overlay keeps;
- the hosted sandbox, through the same one-shot;
- the [Helm chart](kubernetes.md), whose default `definitions.files` lists
  them, stored by an init container. Setting `definitions.files` replaces the
  list, so a deployment that loads its own templates lists those.

## The mapping that names them

The worked example of [the party mapping](config-auth.md#the-party-mapping)
maps the first five, and boots on them as they stand. A test loads the files,
boots that example from the page and provisions a SCIM User, a Group with the
User as its member and a representation through it, so the example and the
files cannot drift apart. SCIM serves `/scim/v2/Groups` once the group table
names a `display_name` and the membership table is set, and each client's
`[[auth.party]]` table needs a `group_claim` and a
`group_identifier_issuer` ([Groups](config-auth.md#groups)).

The quickstart, its Keycloak overlay, the hosted sandbox and the
development lanes map all five, so SCIM Users, Groups and representation
work there out of the box. No configuration key names the patient `PERSON`
or the provider `ORGANISATION`. The Helm chart's party mapping is yours: its
`values-production.yaml` maps the `PERSON` and the `ROLE`, and you add the
rest when you want them.

For the Helm chart, the same tables go under `config.authz.party_binding`:

```yaml
config:
  authz:
    party_binding:
      group:
        template_id: openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0
        external_id: { path: "/details[id2]/items[id3]", type: scim_external_id }
        display_name: "/identities[id4]/details[id5]/items[id6]"
      membership:
        archetype_id: openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1
      representation:
        archetype_id: openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1
        relationship_codes: { at10: "254", at11: "264" }
```

Set the mapping before the first start. The binding side table is filled
under the mapping's digest at the first start, and a server whose mapping
differs from it refuses to start until
[`ferroehr party-binding refill`](config-cli.md#ferroehr-party-binding-refill)
has run with every replica stopped.
