# Default demographic definitions

Party binding and SCIM provisioning write demographic parties under the
templates and archetypes the [party mapping](config-auth.md#the-party-mapping)
names, and the server refuses to boot while the definition store lacks one of
them. FerroEHR ships a default set of seven ADL 2 archetypes. The binary
carries them, every boot stores each one whose id the definition store does
not hold, and with no mapping key set the party mapping names them, so an
empty configuration boots with SCIM Users, Groups and representation
working:

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
not in this set, and are stored at boot the same way.

A stored definition is never overwritten: a definition stored under one of
these ids before the first boot, with its own text, is kept, and so is every
later version uploaded through the definition API. These are defaults. A
deployment may load its own templates instead, or beside them, and set the
`[authz.party_binding]` sections that name them; the server stores nothing
under its ids.

## The files

Each file is in the repository under `deploy/definitions/` and is attached to
every release as an asset of its own, with a `.sha256sum` file beside it.

| File | What it is for | The key that names it |
|---|---|---|
| [`openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls) | A professional's `PERSON`: the binding key, `userName` and `externalId` as identifiers in `details` (`/details[id2]/items[id3]`), the full, given and family name in `identities` (`/identities[id4]/details[id5]/items[id6]`, `id7`, `id8`), and a work email in `contacts` (`/contacts[id9]/addresses[id10]/details[id11]/items[id12]`). | `[[authz.party_binding.person.template]] template_id` and the `path` keys of `[[authz.party_binding.person.attribute]]` |
| [`openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls) | The professional `ROLE` that makes a `PERSON` a professional while it is valid, with a `CAPABILITY` per credential: the field of practice and each registration and qualification, as the CKM `individual_credentials` archetype records them (`/capabilities[id6]/credentials[id7]/items[id10]` is a registration). | `[authz.party_binding.professional_role] template_id` |
| [`openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls) | A care team's `GROUP`: its `externalId` as an identifier in `details` (`/details[id2]/items[id3]`) and its `displayName` in `identities` (`/identities[id4]/details[id5]/items[id6]`). | `[authz.party_binding.group] template_id` |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls) | The membership of a `PERSON` in a `GROUP`, held in the member's `relationships`, with an optional role in the team. | `[authz.party_binding.membership] archetype_id`, as `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1` |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1.0.0.adls) | An example care-team membership from a professional's `PERSON` to the team's `GROUP`, held in the professional's `relationships`, whose details carry `ferroehr-operations::member_of` and an optional role in the team. FerroEHR recognises it by that code in any archetype. | none |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v1.0.0.adls) | An example care relationship from a patient's `PERSON` to the team's `GROUP` or the unit, held in the patient's `relationships`, with the admission as its `time_validity`, whose details carry `ferroehr-operations::under_care_of` and an optional discharge time. FerroEHR recognises it by that code in any archetype. | none |
| [`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls) | An example representation from a representative's `PERSON` to the subject's, whose coded name is its type, `at10` (parent), `at11` (guardian) or `at12` (authorised proxy), and whose details carry `ferroehr-operations::represents` and the openEHR `subject_relationship` code. FerroEHR recognises a representation by that code in any archetype. | none |
| [`openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls`](https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls) | A patient's `PERSON`: its patient identifiers (`/details[id2]/items[id3]`), date of birth (`id4`) and sex (`id5`) in `details`, its names in `identities`, each named by its usage (`/identities[id6]/details[id7]/items[id8]`, `id9`, `id10`), a postal and an electronic address, its relationships to personal contacts, third-party payers and providers, each payer and provider with the patient identifiers it issues, and its care relationships (`id32`) to the teams and units it is under the care of, carrying `ferroehr-operations::under_care_of`. | `[authz.party_binding.patient] template_id`, with its `identifier` and `name` paths |
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
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1.0.0.adls \
  openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v1.0.0.adls \
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
  the `GROUP`, the membership and the two care relationships;
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

## How they are stored

Every boot stores each file whose id the definition store does not hold,
through the same path as `POST /definition/template/adl2`, before it judges
the party mapping. A file stored already is left alone, whatever its text.
No lane loads them with `ferroehr definitions load` any more: the
quickstart, the hosted sandbox, the development and conformance lanes and
the [Helm chart](kubernetes.md) boot on them as the binary stores them. The
chart's `definitions.files` is empty by default and lists only definitions
of your own.

To store them before the first boot, for example to inspect them, use
[`ferroehr definitions load`](config-cli.md#ferroehr-definitions-load) with
the files above; the first boot then keeps them.

## The mapping that names them

With no mapping key set, `[authz.party_binding]` maps all of them but the
provider `ORGANISATION`, which the acts-for relationship of the policy parties
targets ([the default mapping](config-auth.md#the-party-mapping)). SCIM serves
`/scim/v2/Groups`, and each client's Groups' `externalId`s are written under
its `[[auth.party]]` table's `group_identifier_issuer`, or under one derived
from its `binding_issuer` when the table sets none
([Groups](config-auth.md#groups)). A test boots an empty configuration,
stores the set as boot does, and provisions a SCIM User, a Group with the
User as its member and a representation through it, so the default mapping
and the files cannot drift apart.

A deployment with templates of its own sets the section that names them, and
only that section: the others keep their default. For the Helm chart, the
sections go under `config.authz.party_binding`, for example a person key of
another identifier type:

```yaml
config:
  authz:
    party_binding:
      person:
        template:
          - template_id: openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0
            default: true
            key: { path: "/details[id2]/items[id3]", type: uuid }
            user_name: { path: "/details[id2]/items[id3]", type: scim_user_name }
            external_id: { path: "/details[id2]/items[id3]", type: scim_external_id }
```

Set the mapping before the first start. The binding side table is filled
under the mapping's digest at the first start, and a server whose mapping
differs from it refuses to start until
[`ferroehr party-binding refill`](config-cli.md#ferroehr-party-binding-refill)
has run with every replica stopped.
