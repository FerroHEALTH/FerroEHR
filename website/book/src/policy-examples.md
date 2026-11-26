# Worked policy examples

These examples write the policy parties of [Operational policy](operational-policy.md)
for common callers, as the requests an administrator sends.

<!-- toc -->


The examples use the base path `/ferroehr/rest/openehr/v1`, send
`Content-Type: application/json` and `Prefer: return=representation`, and
run as an administrator. The party ids are examples: use the ids your
server returns in the `Location` header and the body.

The provider this instance acts for is an `ORGANISATION` named
`9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40`, and the federation partner below is
the `ORGANISATION` `7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36`. The default set's
healthcare provider `ORGANISATION` (`openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0`,
on [Default demographic definitions](installation/default-demographic-definitions.md))
records both, identified by their provider identifier. Neither is a policy
party, so any demographic writer may create them.

## A system client

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
      "_type": "ELEMENT", "archetype_node_id": "id3",
      "name": {
        "_type": "DV_CODED_TEXT", "value": "client_id",
        "defining_code": {
          "_type": "CODE_PHRASE",
          "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
          "code_string": "client_id"
        }
      },
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
`performer`. Its capability grants `commit_composition` with the origin
`relay`, the template it writes and the facility it writes at, each element
named by its `openehr_access` code. A system that also registers
device measurements holds a second capability with the origin `device` and
its `device_identifier_issuer`.

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
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Relay" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Commit composition", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "commit_composition" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Origin", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "origin" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Relay", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "relay" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Template written", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "template" } },
          "value": { "_type": "DV_TEXT", "value": "openEHR-EHR-COMPOSITION.report-result.v1" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Facility", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "facility" } },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id11",
              "name": { "_type": "DV_CODED_TEXT", "value": "Issuer", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "issuer" } },
              "value": { "_type": "DV_TEXT", "value": "urn:oid:2.16.528.1.1007.3.3" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id12",
              "name": { "_type": "DV_CODED_TEXT", "value": "Identifier", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "identifier" } },
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
      "_type": "ELEMENT", "archetype_node_id": "id3",
      "name": {
        "_type": "DV_CODED_TEXT", "value": "client_id",
        "defining_code": {
          "_type": "CODE_PHRASE",
          "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
          "code_string": "client_id"
        }
      },
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
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Details" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Acts-for code" },
        "value": {
          "_type": "DV_CODED_TEXT", "value": "Acts for",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "acts_for" }
        }
      }]
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

## A backend client and a demographic client

A reporting job queries every EHR with a `system/aql-*` scope, and a master
patient index reads and writes persons through the DEMOGRAPHIC API. Neither
is a key in `ferroehr.toml`: each is a grant of the client's `AGENT`, read on
every request. A `system/aql-*` scope "would grant access to all registered
and ad-hoc AQL queries system-wide" (SMART on openEHR, master08 §Resource
Scopes), so it counts only for a client granted `query_system_wide`; no
SMART on openEHR scope covers demographic data, so the DEMOGRAPHIC API admits
a client granted `access_demographic`, or a caller holding the
`[auth] product_scope`.

Both operations reach beyond one system's own writes, so the ceiling admits
each by name:

```toml
[authz.grants]
allowed = ["commit_composition", "query_system_wide", "access_demographic"]
```

The client's `AGENT` is written as above, and its grant is a `ROLE` it
performs, `POST /demographic/role`. One `ROLE` can carry both operations; a
deployment that runs the two as separate clients gives each its own `AGENT`
and `ROLE`.

<!-- policy-example: backend-client-role -->
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
        "value": { "_type": "DV_TEXT", "value": "Reporting and patient index" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Reporting" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Query system-wide",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "query_system_wide" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Access demographic",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "access_demographic" }
          }
        }
      ]
    }
  }]
}
```

The client's next request runs under the grant, on every replica, with no
restart. The grant counts for the client's own client credentials token,
whose `client_id` the `AGENT` carries. A person signed in
through the same client holds the person's own grants, never the client's.
To withdraw it, end the `ROLE`'s `time_validity` or delete the `ROLE`
([Ending a grant](#ending-a-grant)).

## A migration

A migration is a system client like any other: its `AGENT` is written as
above, and its `ROLE` holds the import and export operations. Here a job
imports the EHRs of a legacy system and its extracts over one night, and may
export them again to check the result. The capability's `time_validity`
holds the migration to two days, `Not before` and `Not after` narrow it to
the night between them, and `subject_namespace` limits it to the legacy
system's patients. `[authz.grants] allowed` must admit `import_ehr`,
`import_ehr_extract` and `export_ehr_extract`.

<!-- policy-example: migration-role -->
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
        "value": { "_type": "DV_TEXT", "value": "Legacy HIS migration" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Migration" },
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
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "import_ehr" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "import_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Export EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "export_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Subject namespace", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "subject_namespace" } },
          "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id15",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not before", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "not_before" } },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id16",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not after", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "not_after" } },
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

The job then calls `POST /message/import` (`message_import_ehr`) and
`POST /message/import/{ehr_id}` (`message_import_ehr_extract`) with its own
token, and `POST /message/export` to read the result back. Each
import is committed under the job's `ACTOR` and is an access record that names
it and the provider it acts for. The versions of an extract keep their legacy
committers when a `creating_system` element of the grant names the legacy
system's `creating_system_id`; otherwise an extract naming a local party is
refused.
When the night ends the grant holds no longer, with no change to anything.

## A federation partner

A partner organisation's professionals read patient summaries under a
data-sharing agreement. The partner is an `ORGANISATION` that carries a
client identifier for each client its `federation` issuer issues tokens to:
an `ELEMENT` in its `details` named `openehr_access::client_id` whose value
is, for example, `{ "_type": "DV_IDENTIFIER", "issuer":
"https://federation.example", "id": "remote-portal" }`. Committing it is an
administrator's write, because the element makes it a policy party. The agreement is a `ROLE` of the
federation partner template the partner performs, with one capability per
accessor, here `person`. Each term is an element whose `name` is its
`openehr_access` code, which is how FerroEHR reads it in any archetype
(see [Federation agreements](operational-policy.md#federation-agreements)).

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
    "lower": { "_type": "DV_DATE", "value": "2026-01-01" },
    "upper": { "_type": "DV_DATE", "value": "2036-12-31" },
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
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Federated person access", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "federated_person_access" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement name", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "agreement_name" } },
          "value": { "_type": "DV_TEXT", "value": "Regional care network exchange" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement version", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "agreement_version" } },
          "value": { "_type": "DV_TEXT", "value": "2.1" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id12",
          "name": { "_type": "DV_CODED_TEXT", "value": "Exchange", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "exchange" } },
          "value": { "_type": "DV_TEXT", "value": "patient summary" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id13",
          "name": { "_type": "DV_CODED_TEXT", "value": "Purpose", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "purpose" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "treatment", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ActReason" }, "code_string": "TREAT" } }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Legal ground" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id15",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 6(1) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "gdpr_art_6_1" } },
              "value": { "_type": "DV_TEXT", "value": "e" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id16",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 9(2) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "gdpr_art_9_2" } },
              "value": { "_type": "DV_TEXT", "value": "h" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id17",
              "name": { "_type": "DV_CODED_TEXT", "value": "National provision", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "national_provision" } },
              "value": { "_type": "DV_TEXT", "value": "the provision the agreement names" }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Consent source", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "consent" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Local", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "local" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Read scope", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "read_scope" } },
          "value": { "_type": "DV_TEXT", "value": "patient/composition-openEHR-EHR-COMPOSITION.patient_summary.v1.r" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id24", "name": { "_type": "DV_TEXT", "value": "Caller rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id25",
              "name": { "_type": "DV_CODED_TEXT", "value": "Requests per second", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "requests_per_second" } },
              "value": { "_type": "DV_COUNT", "magnitude": 5 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id26",
              "name": { "_type": "DV_CODED_TEXT", "value": "Burst", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "burst" } },
              "value": { "_type": "DV_COUNT", "magnitude": 20 }
            }
          ]
        }
      ]
    }
  }]
}
```

## A privileged role

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
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Read subject audit",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "read_subject_audit" }
          }
        },
        {
        "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Authorisation reference" },
        "value": { "_type": "DV_TEXT", "value": "Cover for the privacy officer, approved by the CMIO" }
      }]
    }
  }]
}
```

## A scribe composing for physicians

A scribe records the notes the physicians of the same provider dictate. The
grant is a `ROLE` of the privileged template, version `1.0.1`, whose kind
`at14` is `Compose for`, performed by the scribe's `PERSON`
`3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95`. Each `COMPOSITION` the scribe
commits names the physician as its composer, and the committer is the scribe.

<!-- policy-example: compose-for-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.1" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Privileged role" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Medical scribe" }
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
      "_type": "DV_CODED_TEXT", "value": "Compose for",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at14" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
        "value": {
          "_type": "DV_CODED_TEXT", "value": "Compose for",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "compose_for" }
        }
      }]
    }
  }]
}
```

Allow the operation by name: `[authz.grants] allowed = ["commit_composition", "compose_for"]`.

## Just-in-time admission

A first login of a person no party carries creates a provisional `PERSON`
only when a `create_at_login` grant admits it. The grant is a `ROLE` like
any other, and FerroEHR reads it on every first login, whoever performs the
`ROLE`: here the provider `ORGANISATION` performs it, which reads as "the
hospital admits its own staff". Its capability names the issuer by
`login_issuer` and declares the admission conditions:

- `admit_idp`: the identity providers whose logins are created, read from
  the issuer's `[[authz.party_binding.issuer]] idp_claim` (`idp` by
  default);
- `admit_group`: the groups, read from the `group_claim` of the issuer's
  party table, one of which an admitted login holds;
- `admit_role`: the roles, read from `[auth] role_claims`, one of which an
  admitted login holds;
- `admit_assurance`: the lowest `[authz.assurance]` level an admitted login
  carries.

A login is admitted when it meets every condition the grant declares; each
list admits any one of its values. Several grants in force admit a login
when any one of them does. A grant names at least one `login_issuer` and
at least one of `admit_idp`, `admit_group`, `admit_role` and
`admit_assurance`, so no grant admits every login of an issuer: a grant
without a condition is refused with `422`, as is an element whose value
does not read. `[authz.grants] allowed` must list `create_at_login`, which
the default does not.

The provider admits the logins of its Microsoft Entra tenant from members of
one group at assurance substantial or above, from 1 December 2026,
`POST /demographic/role`:

<!-- policy-example: login-admission -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.login_admission.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.login_admission.v1" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Login admission" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "at0003", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Staff first login" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
    "id": { "_type": "HIER_OBJECT_ID", "value": "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2026-12-01" },
    "lower_unbounded": false, "upper_unbounded": true,
    "lower_included": true, "upper_included": false
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "at0010", "name": { "_type": "DV_TEXT", "value": "Admission" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "at0011", "name": { "_type": "DV_TEXT", "value": "Terms" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "at0012", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Create at login",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "create_at_login" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0013",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Login issuer",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "login_issuer" }
          },
          "value": { "_type": "DV_URI", "value": "https://login.microsoftonline.com/0f1e2d3c-4b5a-4968-8776-655443322110/v2.0" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0014",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted group",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "admit_group" }
          },
          "value": { "_type": "DV_TEXT", "value": "5f2a9c1e-0000-4000-8000-00000000c11a" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0015",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted assurance",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "admit_assurance" }
          },
          "value": { "_type": "DV_TEXT", "value": "substantial" }
        }
      ]
    }
  }]
}
```

The `ROLE` is a policy party, so only an administrator writes or reads it.
The next first login of that tenant that meets both conditions creates its
`PERSON`, on every replica and with no restart. To stop creating, end the
`ROLE` as below; a `PERSON` already created stays. The configuration keeps
only how the issuer's tokens are read
([Just-in-time creation](installation/config-authz.md#just-in-time-creation)).

## Ending a grant

To end a grant, an administrator commits a new version of the `ROLE` whose
`time_validity` ends before today (an upper bound of yesterday, or of today
with `upper_included` false), with `PUT /demographic/role/{uid}` and the
`If-Match` of its current version, or deletes it with
`DELETE /demographic/role/{uid}`. Both are versions with their committer, so
the revision history keeps the grant and who ended it, and the change decides
the caller's next request.
