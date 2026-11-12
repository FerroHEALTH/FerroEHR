# Operational policy

A hospital CDR runs around the clock, so a change to who may do what cannot
wait for a restart. FerroEHR holds its operational policy as openEHR
demographic data in its own party store: which systems call it, what each
may write, import or export, the terms of each federation partner, and the
privileges a professional holds beyond clinical access. You manage that
policy through the Demographic API. Every change is a new version with its
committer, so the version history of a policy party is the history of the
grant.

This page describes the model, the definitions FerroEHR ships for it, the
rule that only an administrator writes and reads a policy party, and a
worked example of each kind of policy as the JSON bodies you send. FerroEHR
stores, validates and protects the policy parties; the server does not yet
read a grant from them, so the request path does not honour one.

No openEHR specification gives a party an authorization meaning. The classes
are the openEHR Reference Model's (RM Demographic Information Model); how
FerroEHR reads them as policy is its own design, and the points that are its
own are listed under [Own design](#own-design).

## What stays in the configuration

The configuration file keeps what the server needs before it can reach its
store, or before it can trust anything:

- the listeners, the database, TLS, telemetry, the audit sinks and record
  signing;
- the token issuers with their keys and trust bounds, `[auth] product_scope`,
  and `[authz.rbac] admin_role` with `role_issuers`: the authority that
  writes policy parties;
- the party mapping of the deployment's own people (`[authz.party_binding]`),
  which is digest-locked; the policy parties take no key;
- the provider the instance acts for (`[authz.party_binding] provider`);
- the security floors: the assurance minimum, `ehr_access_default` and the
  cohort thresholds;
- break-glass: `[authz.rbac] emergency_access_role` and
  `[audit] emergency_purpose_codes`, which must work as configured when the
  party store does not answer;
- the password hashes of the Basic users, since a secret never becomes
  versioned openEHR data;
- the grant ceiling, `[authz.grants] allowed`: the kinds of grant the party
  store may hold at all, so a compromised administrator account cannot grant
  more than the operator allows. The key arrives with the request path that
  reads the grants.

Everything else about who may do what is party data.

## The model

**The actor.** A system is an `AGENT` ("any kind of agent, including
devices, software systems", RM `AGENT`), an organisation is an
`ORGANISATION` and a person is a `PERSON`. Each is identified by a
`DV_IDENTIFIER` in `PARTY.details` (RM §Party Identification: identifiers
given by organisations "are treated as any other attribute of a Party").
A system's `AGENT` carries its OAuth 2.0 `client_id` under the token
issuer: the identifier's `issuer` is the issuer URL, its `id` the
`client_id` and its `type` `client_id`.

**What it may do.** The actor performs a `ROLE` (RM `ROLE.performer`), and
the role's `CAPABILITY` holds the grant in its `credentials`. The openEHR RM
and CKM use a capability for a performer's qualifications ("Capability
should be backed up by credentials", RM `CAPABILITY`), and the professional
`ROLE` of the default set records a clinician's registrations that way,
from the CKM `individual_credentials` archetype. Reading a capability as an
authorization grant is FerroEHR's own design, and the capability of each
policy `ROLE` is FerroEHR's own archetype. The role's
`time_validity` bounds the engagement and the capability's `time_validity`
bounds the grant. Both are `DV_INTERVAL<DV_DATE>`, so a bound finer than a
day goes in the credentials, as the `Not before` and `Not after` elements.

**On whose behalf.** A system holds an acts-for `PARTY_RELATIONSHIP` from its
`AGENT` to the provider's `ORGANISATION`. The RM stores a relationship in
the party at its source (RM §Party Relationships), so the acts-for
relationship is part of the `AGENT`'s version.

**Management.** The policy parties are written through the standard
ITS-REST 1.1.0 Demographic API, which marks itself `DEVELOPMENT`, and
FerroEHR adds no endpoint of its own for them: `POST`, `PUT` and `DELETE` on
`/demographic/agent`, `/demographic/organisation`, `/demographic/person` and
`/demographic/role`, and `POST /demographic/contribution` for a change of
several parties in one commit.
`GET /demographic/versioned_party/{id}/revision_history` is the history of a
grant. A `PARTY_RELATIONSHIP` changes through an update of its source party,
since the RM stores it there.

**Audit.** Every change carries its committer in
`AUDIT_DETAILS.committer` and is an access record of the demographic API, so
who granted what, and when, is recorded twice.

**Revocation.** A grant ends with a new version of the `ROLE` whose
`time_validity` ends today, or with its deletion. An `AGENT` that committed
versions is set inactive, never deleted, because it is their committer.

## Who writes and reads a policy party

The rule keys on the archetype a party is built on, so a renamed, versioned
or specialised template is guarded too. A policy party is:

- an `AGENT` whose root archetype is the system `AGENT` archetype or a
  specialisation of it, under any template;
- a `ROLE` whose root archetype is one of the four policy `ROLE` archetypes
  or a specialisation, under any template;
- any party holding an acts-for relationship, and an acts-for
  `PARTY_RELATIONSHIP` written on FerroEHR's own relationship routes.

Only a caller holding `[authz.rbac] admin_role` creates, updates or deletes
one, through its own route or in a `POST /demographic/contribution`. Any
other caller is answered `403`, and the answer names the archetype, for
example `the AGENT … is a policy party of the archetype
openehr-demographic-agent.system.v1: only an administrator writes it`. A
SCIM client never writes one, even for a `PERSON` it provisioned, and
neither does a holder of a `demographic_clients` grant, so no client grants
itself a capability. The rule judges the version before the write as well
as the new one, so a non-administrator can neither change nor delete a
policy party, nor turn one into an ordinary party.

Only an administrator reads one too: `GET` of the party, of
`/demographic/versioned_party/{id}` and its `revision_history` and
`version` reads, and of its tags answer `403` to any other caller, naming
the archetype, when any version of the party is a policy party. Every
other party stays readable as before. AQL over the demographic store does
not apply this rule yet ([#5450](https://github.com/FerroHEALTH/FerroEHR/issues/5450)).

Every write and every read is an access record of the Demographic API, with
its committer, as any demographic access is.

## The policy archetypes

FerroEHR recognises a policy party by its root archetype, never by its
template. A party is a system `AGENT`, a policy `ROLE` or an acts-for
relationship when its root archetype, or the archetype of a relationship it
holds, is one of the archetypes below or an ADL 2 specialisation of one
(`openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1` specialises
`openEHR-DEMOGRAPHIC-ROLE.system_client.v1`), at any minor version of the
same major version. You version, rename or constrain the templates built on
these archetypes, or build your own, and they stay policy parties: openEHR
expects data of a specialised archetype to conform to its parent (AM ADL 2
§Specialisation). No configuration key names them.

| Kind | Archetype |
|---|---|
| System `AGENT` | `openEHR-DEMOGRAPHIC-AGENT.system.v1` |
| Acts-for relationship | `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1` |
| System client `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.system_client.v1` |
| Data migration `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.data_migration.v1` |
| Federation partner `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1` |
| Privileged `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.privileged.v1` |

**Default definitions.** The binary carries a default template of each,
version `1.0.0` (for example `openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0`).
At boot FerroEHR stores each one through the normal definition path only
when no definition of its id is stored. It never overwrites a stored
definition, never refuses one you changed, and never seeds over your change.
After that they are ordinary definitions: upload a new version, a renamed or
a specialised template through the definition API, as for any other. Every
write of a policy party is validated against its own template. The sources
and every change from them are in
`app/ferroehr/assets/definitions/policy/PROVENANCE.md`.

**The contract.** The archetype ids above and the node ids below are what
FerroEHR reads, and every template and version built on an archetype keeps
them. A breaking change to them is a new archetype major version (`.v2`),
which FerroEHR does not take for the `.v1` kind. Each grant is a
`CAPABILITY` at `/capabilities[id6]`, its kind the coded `name`, its
credentials at `/capabilities[id6]/credentials[id8]`, and its `time_validity` the day bounds.

| Archetype | Field | Node path | Type |
|---|---|---|---|
| `AGENT.system` | Client identifier (`issuer` the token issuer, `id` the `client_id`, `type` `client_id`) | `/details[id2]/items[id3]` | `DV_IDENTIFIER` |
| `AGENT.system` | Name | `/identities[id4]/details[id5]/items[id6]` | `DV_TEXT` |
| `PARTY_RELATIONSHIP.acts_for` | Type (its `name`) | `at10` acts for | `DV_CODED_TEXT` |
| `PARTY_RELATIONSHIP.acts_for` | Agreement reference | `/details[id3]/items[id4]` | `DV_TEXT` |
| every policy `ROLE` | Kind of grant (the capability's `name`) | `/capabilities[id6]/name` | `DV_CODED_TEXT` |
| `ROLE.system_client` | Kind | `at10` relay, `at11` device | |
| `ROLE.system_client` | Template written | `/capabilities[id6]/credentials[id8]/items[id9]` | `DV_TEXT` |
| `ROLE.system_client` | Facility: issuer, identifier, identifier prefix | `/capabilities[id6]/credentials[id8]/items[id10]/items[id11]`, `[id12]`, `[id13]` | `DV_TEXT` |
| `ROLE.system_client` | Device identifier issuer | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.system_client` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.data_migration` | Kind | `at10` data migration | |
| `ROLE.data_migration` | Operation | `/capabilities[id6]/credentials[id8]/items[id9]`: `at20` import EHR, `at21` import EHR extract, `at22` export EHR extract | `DV_CODED_TEXT` |
| `ROLE.data_migration` | Subject namespace | `/capabilities[id6]/credentials[id8]/items[id10]` | `DV_TEXT` |
| `ROLE.data_migration` | Rate: requests per second, burst | `/capabilities[id6]/credentials[id8]/items[id11]/items[id12]`, `[id13]` | `DV_COUNT` |
| `ROLE.data_migration` | Trusted creating system | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.data_migration` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.federation_partner` | Kind (accessor) | `at10` person, `at11` organisation | |
| `ROLE.federation_partner` | Agreement name, version, jurisdiction, exchange | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id11]`, `[id12]` | `DV_TEXT` |
| `ROLE.federation_partner` | Purpose | `/capabilities[id6]/credentials[id8]/items[id13]`: `at20` care, `at21` keuring | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Legal ground: GDPR Art. 6(1), Art. 9(2), national provision | `/capabilities[id6]/credentials[id8]/items[id14]/items[id15]`, `[id16]`, `[id17]` | `DV_TEXT` |
| `ROLE.federation_partner` | Consent source | `/capabilities[id6]/credentials[id8]/items[id18]`: `at30` Mitz, `at31` local | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Read scope, template written | `/capabilities[id6]/credentials[id8]/items[id19]`, `[id20]` | `DV_TEXT` |
| `ROLE.federation_partner` | Discard, discard window | `/capabilities[id6]/credentials[id8]/items[id21]`, `[id22]` | `DV_BOOLEAN`, `DV_DURATION` |
| `ROLE.federation_partner` | MyHealth@EU service | `/capabilities[id6]/credentials[id8]/items[id23]`: `at40` Patient Summary, `at41` ePrescription, `at42` original clinical document | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Caller rate: requests per second, burst | `/capabilities[id6]/credentials[id8]/items[id24]/items[id25]`, `[id26]` | `DV_COUNT` |
| `ROLE.federation_partner` | Opt-out override | `/capabilities[id6]/credentials[id8]/items[id27]` | `DV_BOOLEAN` |
| `ROLE.federation_partner` | Access class | `/capabilities[id6]/credentials[id8]/items[id28]`: `at50` automated, `at51` chained | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Chained log retention | `/capabilities[id6]/credentials[id8]/items[id29]` | `DV_DURATION` |
| `ROLE.federation_partner` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id30]`, `[id31]`, `[id32]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.privileged` | Kind (privilege) | `at10` subject audit, `at11` access service, `at12` identifier reader, `at13` import | |
| `ROLE.privileged` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id11]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |

## Worked examples

The examples use the base path `/ferroehr/rest/openehr/v1`, send
`Content-Type: application/json` and `Prefer: return=representation`, and
run as an administrator. The party ids are examples: use the ids your
server returns in the `Location` header and the body.

The provider this instance acts for is an `ORGANISATION` named
`9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40`, and the federation partner below is
the `ORGANISATION` `7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36`. The default set's
healthcare provider `ORGANISATION` (`openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0`,
on [Default demographic definitions](default-demographic-definitions.md))
records both, identified by their provider identifier. Neither is a policy
party, so any demographic writer may create them.

### A system client

A laboratory relay commits laboratory results into EHRs at one facility.
First its `AGENT`, `POST /demographic/agent`:

<!-- policy-example: agent -->
```json
{
  "_type": "AGENT",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.system.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System agent" },
  "details": {
    "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Identifiers" },
    "items": [{
      "_type": "ELEMENT", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Identifier" },
      "value": {
        "_type": "DV_IDENTIFIER",
        "issuer": "https://login.example.org/machines",
        "id": "lab-relay",
        "type": "client_id"
      }
    }]
  },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": { "_type": "DV_TEXT", "value": "Items" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay" }
      }]
    }
  }]
}
```

Its grant is a `ROLE` it performs, `POST /demographic/role`, with the
`AGENT` `5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73` the first call returned as
`performer`. One capability per origin, here `relay`: the templates it
writes and the facility it writes at.

<!-- policy-example: system-client-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System client" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay writes" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2026-12-01" },
    "lower_unbounded": false, "upper_unbounded": true,
    "lower_included": true, "upper_included": false
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Relay",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Template written" },
          "value": { "_type": "DV_TEXT", "value": "openEHR-EHR-COMPOSITION.report-result.v1" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Facility" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Issuer" },
              "value": { "_type": "DV_TEXT", "value": "urn:oid:2.16.528.1.1007.3.3" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Identifier" },
              "value": { "_type": "DV_TEXT", "value": "01234567" }
            }
          ]
        }
      ]
    }
  }]
}
```

Then a new version of the `AGENT`, `PUT /demographic/agent/{uid}` with the
`If-Match` of its version, that names the `ROLE` it performs and the
provider it acts for. A relationship is stored in its source party, and the
`AGENT`'s id exists only once the first call returns, so the acts-for
relationship goes in this second version.

<!-- policy-example: agent-acts-for -->
```json
{
  "_type": "AGENT",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.system.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System agent" },
  "details": {
    "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Identifiers" },
    "items": [{
      "_type": "ELEMENT", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Identifier" },
      "value": {
        "_type": "DV_IDENTIFIER",
        "issuer": "https://login.example.org/machines",
        "id": "lab-relay",
        "type": "client_id"
      }
    }]
  },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": { "_type": "DV_TEXT", "value": "Items" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay" }
      }]
    }
  }],
  "roles": [{
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ROLE",
    "id": { "_type": "HIER_OBJECT_ID", "value": "e41c7b2a-6d3f-4a9e-8b05-2f7d1c9e3a68" }
  }],
  "relationships": [{
    "_type": "PARTY_RELATIONSHIP",
    "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Acts for",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "source": {
      "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
      "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
    },
    "target": {
      "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
      "id": { "_type": "HIER_OBJECT_ID", "value": "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40" }
    }
  }]
}
```

### A data migration

A migration job imports the EHRs of a legacy system and its extracts over
one night. Its `AGENT` is written as above; its grant is a `ROLE` of the
data migration template, `POST /demographic/role`. The capability's
`time_validity` holds the migration to two days, and `Not before` and
`Not after` narrow it to the night between them.

<!-- policy-example: data-migration-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Data migration" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Legacy HIS migration" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Data migration",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "time_validity": {
      "_type": "DV_INTERVAL",
      "lower": { "_type": "DV_DATE", "value": "2026-12-05" },
      "upper": { "_type": "DV_DATE", "value": "2026-12-06" },
      "lower_unbounded": false, "upper_unbounded": false,
      "lower_included": true, "upper_included": true
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at20" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR extract",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at21" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Subject namespace" },
          "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Requests per second" },
              "value": { "_type": "DV_COUNT", "magnitude": 50 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id13", "name": { "_type": "DV_TEXT", "value": "Burst" },
              "value": { "_type": "DV_COUNT", "magnitude": 100 }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Trusted creating system" },
          "value": { "_type": "DV_TEXT", "value": "legacy-his.example.org" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id15", "name": { "_type": "DV_TEXT", "value": "Not before" },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id16", "name": { "_type": "DV_TEXT", "value": "Not after" },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-06T05:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id17", "name": { "_type": "DV_TEXT", "value": "Authorisation reference" },
          "value": { "_type": "DV_TEXT", "value": "CHG-2026-1182" }
        }
      ]
    }
  }]
}
```

### A federation partner

A partner organisation's professionals read patient summaries under a
data-sharing agreement. The grant is a `ROLE` of the federation partner
template performed by the partner's `ORGANISATION`, with one capability per
accessor, here `person`.

<!-- policy-example: federation-partner-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Federation partner" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Regional care network" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
    "id": { "_type": "HIER_OBJECT_ID", "value": "7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2027-01-01" },
    "upper": { "_type": "DV_DATE", "value": "2027-12-31" },
    "lower_unbounded": false, "upper_unbounded": false,
    "lower_included": true, "upper_included": true
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Person",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Agreement name" },
          "value": { "_type": "DV_TEXT", "value": "Regional care network exchange" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Agreement version" },
          "value": { "_type": "DV_TEXT", "value": "2.1" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Jurisdiction" },
          "value": { "_type": "DV_TEXT", "value": "NL" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Exchange" },
          "value": { "_type": "DV_TEXT", "value": "patient summary" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id13", "name": { "_type": "DV_TEXT", "value": "Purpose" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Care",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at20" }
          }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Legal ground" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id15", "name": { "_type": "DV_TEXT", "value": "GDPR Article 6(1) point" },
              "value": { "_type": "DV_TEXT", "value": "c" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id16", "name": { "_type": "DV_TEXT", "value": "GDPR Article 9(2) point" },
              "value": { "_type": "DV_TEXT", "value": "h" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id17", "name": { "_type": "DV_TEXT", "value": "National provision" },
              "value": { "_type": "DV_TEXT", "value": "UAVG Art. 30(3)(a)" }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18", "name": { "_type": "DV_TEXT", "value": "Consent source" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Mitz",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at30" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19", "name": { "_type": "DV_TEXT", "value": "Read scope" },
          "value": { "_type": "DV_TEXT", "value": "patient/composition-openEHR-EHR-COMPOSITION.patient_summary.v1.r" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id24", "name": { "_type": "DV_TEXT", "value": "Caller rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id25", "name": { "_type": "DV_TEXT", "value": "Requests per second" },
              "value": { "_type": "DV_COUNT", "magnitude": 5 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id26", "name": { "_type": "DV_TEXT", "value": "Burst" },
              "value": { "_type": "DV_COUNT", "magnitude": 20 }
            }
          ]
        }
      ]
    }
  }]
}
```

### A privileged role

A professional covers a colleague's access-log reviews for one week. The
grant is a `ROLE` of the privileged template performed by the professional's
`PERSON` `3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95`, with one capability per
privilege, here `subject audit`. The privileged role never grants the
administrator role or emergency access: the identity provider and the
configuration grant those.

<!-- policy-example: privileged-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Privileged role" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Access log review cover" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
    "id": { "_type": "HIER_OBJECT_ID", "value": "3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Subject audit",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "time_validity": {
      "_type": "DV_INTERVAL",
      "lower": { "_type": "DV_DATE", "value": "2026-12-07" },
      "upper": { "_type": "DV_DATE", "value": "2026-12-13" },
      "lower_unbounded": false, "upper_unbounded": false,
      "lower_included": true, "upper_included": true
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Authorisation reference" },
        "value": { "_type": "DV_TEXT", "value": "Cover for the privacy officer, approved by the CMIO" }
      }]
    }
  }]
}
```

### Ending a grant

To end a grant, an administrator commits a new version of the `ROLE` whose
`time_validity` upper bound is today, with `PUT /demographic/role/{uid}` and
the `If-Match` of its current version, or deletes it with
`DELETE /demographic/role/{uid}`. Both are versions with their committer, so
the revision history keeps the grant and who ended it.

## Own design

No openEHR specification governs these points; each is FerroEHR's own
design:

- **A capability is a grant.** The RM defines `CAPABILITY.credentials` as
  "the qualifications of the performer of the role for this capability";
  FerroEHR reads them as what the performer may do.
- **Bounds finer than a day go in the credentials.** `time_validity` is
  `DV_INTERVAL<DV_DATE>` on `ROLE`, `CAPABILITY` and `PARTY_RELATIONSHIP`,
  so `Not before` and `Not after` are `DV_DATE_TIME` elements.
- **Only an administrator writes and reads a policy party.** The RM's
  `ACCESS_CONTROL_SETTINGS` is implementation dependent, and the Demographic
  API defines no access rule for its own operations.
- **The grant ceiling.** `[authz.grants] allowed` bounds what the party store
  may grant.
- **Break-glass stays in the configuration**, so emergency access works when
  the party store does not answer.
