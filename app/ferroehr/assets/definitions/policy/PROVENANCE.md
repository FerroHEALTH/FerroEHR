# Policy definitions: provenance

Six ADL 2 archetypes, the defaults of FerroEHR's authorization model. The
binary carries them (`ferroehr::party_binding::policy::DEFINITIONS`), and
each boot stores one when no definition of its id is stored, never over a
stored one. FerroEHR recognises a policy party by these archetypes (or a
specialisation of one) under any template, and no configuration key names
them; a deployment versions or replaces the templates through the
definition API. A party of one of them is a
policy party, which only an administrator writes and reads; the book's
"Operational policy" page describes the model.

1. `openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0.adls`: the `AGENT` of a software
   system, identified by a `DV_IDENTIFIER` in `details` whose issuer is the
   token issuer and whose id is its OAuth 2.0 `client_id`, at
   `/details[id2]/items[id3]`, type `client_id`.
2. `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0.adls`: the
   relationship from a system's `AGENT` to the provider's `ORGANISATION` it
   acts for, held in the `AGENT`'s `relationships`, recognised in its
   interface form, `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1`.
3. `openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0.adls`: what a system
   commits, one `CAPABILITY` per origin (relay or device).
4. `openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0.adls`: a migration's
   operations, subject namespaces, rate and trusted creating systems.
5. `openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0.adls`: the terms of a
   partner's data-sharing agreement, one `CAPABILITY` per accessor (person or
   organisation).
6. `openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0.adls`: a privilege a person
   holds beyond clinical access (subject audit, access service, identifier
   reader, import).

## Source

openEHR CKM publishes no `AGENT` archetype, no `PARTY_RELATIONSHIP`
archetype and no archetype for an authorization grant. The sources below are
the CKM ADL 1.4 exports vendored verbatim under `corpus/archetypes/ckm/adl14/`
by `scripts/vendor/ckm-archetypes.sh` (provenance in
`corpus/archetypes/ckm/PROVENANCE.md`). Every source is a v0 draft that can change without a new id, so
`other_details["derived_from"]` names each with its CKM citeable id and
its modification time.

| Definition | CKM sources (CKM id, modified) |
|---|---|
| `AGENT` | `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` (1013.1.475, 2019-08-09; an `ACTOR`, like `AGENT`), `openEHR-DEMOGRAPHIC-PARTY_IDENTITY.organisation_name.v0` (1013.1.476, 2019-08-09) |
| acts-for `PARTY_RELATIONSHIP` | the `PARTY_RELATIONSHIP` that `openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0` (1013.1.475, 2019-08-09) constrains inline |
| system client, data migration, privileged `ROLE`s | `openEHR-DEMOGRAPHIC-ROLE.individual_provider.v0` (1013.1.482, 2019-08-09), for the `ROLE` with its name in `identities` |
| federation partner `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.healthcare_provider_organisation.v0` (1013.1.501, 2019-08-09), the `ROLE` of an organisation acting as a healthcare provider, since its performer is the partner's `ORGANISATION` |

## Our own modelling

- **The `CAPABILITY` of each `ROLE` is our own design.** The openEHR RM
  defines `CAPABILITY.credentials` as "the qualifications of the performer of
  the role for this capability", and CKM's
  `openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0` records a
  person's professional registrations and qualifications. Here the
  credentials hold an authorization grant, which no openEHR specification
  or CKM archetype defines. The grant fields of each kind are FerroEHR's
  own: the templates written, the facilities, the operations, the subject
  namespaces, the rate, the terms of an agreement and the privilege kinds.
- **Bounds finer than a day.** `time_validity` is `DV_INTERVAL<DV_DATE>` on
  `ROLE`, `CAPABILITY` and `PARTY_RELATIONSHIP`, so every grant carries
  optional `Not before` and `Not after` elements of type `DV_DATE_TIME`, and
  an optional `Authorisation reference`.
- **The `AGENT` carries its identifier.** Its `details` are mandatory, since
  the identifier is what names the system.
- **The value sets** (origins, operations, accessors, purposes, consent
  sources, MyHealth@EU services, access classes, privileges) are FerroEHR's
  own, and so is the name of the acts-for relationship, its type (RM
  `PARTY_RELATIONSHIP` `Type_validity`).
- The node ids are ADL 2 id-codes and the terminology is English only.

They were written by hand, never produced by FerroEHR's ADL 1.4 to ADL 2
converter.

## Licence

The CKM sources state "This work is licensed under the Creative Commons
Attribution-ShareAlike 3.0 License" with copyright "© openEHR Foundation".
These adaptations carry the same licence and copyright line
(`LICENSE-CC-BY-SA-3.0`), recorded in `REUSE.toml`.
