# Who is the committer

Every version FerroEHR stores carries an `AUDIT_DETAILS`, and its `committer`
is the "identity and optional reference into identity management service, of
user who committed the item" (RM common, `AUDIT_DETAILS`). FerroEHR writes
that committer itself, from the caller it authenticated and the demographic
party it bound the caller to. A client no longer says who committed. This
page shows what the server writes for each kind of caller, what a client may
still send, the refusals, and how a professional's party gets the login
identifier the binding matches.

<!-- toc -->

## Why the server writes it

- **RM common master04 §Referring to Demographic Entities:**
  `PARTY_PROXY.external_ref` "will be non-Void" "where there is an entry in
  such a service for the party in question". A professional is a `PERSON` in
  FerroEHR's demographic store, so the committer names that `PERSON`.
- **RM common master04 §Audit Details** lets the committer carry "a system
  login identifier", and §Attestation names `AUDIT_DETAILS.committer` as "the
  identity of the attesting party".
- **EHDS** (Regulation (EU) 2025/327): Art. 13(4) asks the record to
  "identify the health professional and healthcare provider that carried out
  such registration or update", Annex II 3.1 asks for "reliable mechanisms
  for the identification and authentication of health professionals", and
  Annex II 3.2(a) and (b) ask the access record to name the provider and the
  natural person.

No openEHR specification governs how a login is matched to a party, when the
match is made, or what happens when none matches. That is FerroEHR's own
design, configured under
[`[authz.party_binding]`](../installation/config-auth.md#authzparty_binding).

## The committer for each kind of caller

`ferrohealth-smart` classifies every token by its issuer's `kind` and profile
([Who the caller is](../identity-providers.md#who-the-caller-is)). The server
then writes the committer from the bound party:

| Caller | Committer the server writes |
|---|---|
| A person, at a `persons` issuer through a first-party client | `PARTY_IDENTIFIED` with the `PERSON`'s name, an `external_ref` to the `PERSON`, the login identifier `{issuer = <token iss>, id = <person_key value>, type = "oauth2"}` and the provider identifier `{issuer, id, type = "healthcare-provider"}` |
| A delegated person (a client or RFC 8693 actor in the issuer's `delegates`) | the person's own committer, as above; the client and the actor go into the access record, never into the committer |
| A Basic user | the committer of the `PERSON` its `[[authz.party_binding.basic_user]]` table maps it to, with the login identifier `{issuer = "ferroehr", id = <user>, type = "basic"}` |
| A representative | `PARTY_RELATED` naming the representative's own `PERSON` in `external_ref`, their login identifier, and the coded `relationship` from the openEHR `subject_relationship` group (`253\|unknown\|` when no code fits) |
| A patient of a `patients` issuer | `PARTY_SELF` with no `external_ref` (RM common master04, the first scheme of §Referring to the Patient) |
| A verified external person of a `federation` issuer | `PARTY_IDENTIFIED` with no `external_ref`: the asserted name, the professional's `hp_identifier` under `"<country>:<issuing authority>"`, and the token's `iss` and `sub` with type `oauth2` |
| A MyHealth@EU contact point's professional | as the external person, with the HP Identity Assertion's `NameID` under the assertion's issuer and type `hp_identifier:<NameID format>` |
| A declared system client (`relay` or `device`) | `PARTY_IDENTIFIED` named by the client, with `{issuer = <token iss>, id = <client>, type = "client"}` and no `external_ref` |
| An organisation of a `federation` issuer | none: it commits nothing |
| An undeclared system client | none: it commits nothing into an EHR (`403`) |

A person's version names the provider the deployment acts for, from
`[authz.party_binding] provider`, so the version carries the Art. 13(4)
provider without the access record. A `PERSON` with no name gives a committer
with no `name`.

A system client commits clinical content only as declared:

- a `relay` client relays another system's record of a human act, such as an
  HL7 v2 message. The COMPOSITION names the professional in `composer` or in
  a context participation's `performer`, and the committer stays the client
  (EHDS Art. 13(4)). FerroBRIDGE writes its HL7 v2 messages this way, with the
  clinician in the content and the facility from its mapping rules;
- a `device` client registers a device act with no professional acting. The
  COMPOSITION names the device in `composer`. Treating such a registration as
  outside Art. 13(4) is FerroEHR's reading of that article and of
  `COMPOSITION.composer`;
- either one names a facility its grant covers in
  `context.health_care_facility`, or, without a context, in
  `feeder_audit.originating_system_audit.location`. The server checks the
  facility and never fills it.

The client's grant is policy data in the CDR, and the committer carries an
`external_ref` to the client's `ACTOR`. The full rules are under
[What a system client may do](../operational-policy.md#what-a-system-client-may-do).

## What a client may still send

A client keeps `change_type`, `description` and `lifecycle_state`, and its
COMPOSITION content, the `composer` included. Where ITS-REST requires a
committer (a CONTRIBUTION's `audit`, each member's `commit_audit`, an
embedded attestation), send the placeholder, which names the authenticated
caller and nothing else:

```json
{
  "_type": "PARTY_IDENTIFIED",
  "identifiers": [
    { "_type": "DV_IDENTIFIER", "issuer": "<token iss, or ferroehr>", "id": "<sub, or the Basic user>" }
  ]
}
```

The server replaces it with the committer above. A committer that states
nothing the server's does not (the same name, the caller's own login
identifier, the bound `PERSON` as `external_ref`) is accepted too. On the
header path, send no `openEHR-AUDIT_DETAILS` committer at all.

## The refusals

| Request | Answer |
|---|---|
| A stated committer with another name, another identifier or another `external_ref` | `403` naming `AUDIT_DETAILS.committer` |
| A placeholder naming anyone other than the caller | `403` |
| A stated `PARTY_RELATED`, or a `PARTY_SELF` from anyone but the record's person | `403` |
| A person's committer stated by a system caller | `403` |
| An attestation from a patient, a representative, a system caller, an external caller or an unbound caller | `403`; only a person bound to a professional `PERSON` attests (RM common master04 §Attestation, "an explicit signing by one healthcare agent"; limiting it to professionals is FerroEHR's reading) |
| A commit by an undeclared system client | `403` |
| A system COMPOSITION outside its `writes`, or a person act from a client that is not `relay` | `403` |
| A system COMPOSITION without a facility of its `facilities`, a relay act naming no professional, or an organisation `composer` | `422` naming the element |
| A patient-context token's demographic write | `403` |
| An import of a version whose committer names a local party, from a system `[authz.import] trusted_systems` does not list | `403` |

A login the binding cannot use is refused before any commit; the codes are
under [How a login is looked up](../installation/config-auth.md#how-a-login-is-looked-up).

### The deviation from ITS-REST

The ITS-REST overview (`Requests_and_responses.md`, §openehr-version and
openehr-audit-details) recommends that clients provision the audit headers
"based on authentication and authorization runtime data", and says a supplied
value "MUST be merged". Refusing a supplied committer that names someone
other than the authenticated caller deviates from that merge rule, on
purpose: a merged committer would let a client record another person as the
author, which RM common master04 and EHDS Art. 13(4) rule out. The
deviation is reported upstream in
[#5019](https://github.com/FerroHEALTH/FerroEHR/issues/5019).

## Imports

`POST /message/import…` needs the role `[authz.rbac] import_role` names, an
`import_ehr` or `import_ehr_extract` policy grant, or a `system/` caller. An imported version keeps its wrapped `commit_audit` and
attestations as received, so an `IMPORTED_VERSION` names who committed at the
source. A wrapped committer that names a `PERSON` of this server, or carries
a login identifier of its issuers, is refused with `403` unless the version
was created by a system `[authz.import] trusted_systems` lists.

## What the access record adds

The committer says who committed. The access record of the same request says
how the caller was bound: `binding` (`bound`, `provisional`, `external`,
`organisation`, `system`, `self` or a `refused-*` code), the party, its
groups, the provider, the actor chain of a delegated token and the client. The
values and their renderings are under
[The binding outcome](../audit.md#the-binding-outcome).

## How a party gets its login identifier

The binding matches the token's `person_key` claim against the identifier at
the binding-key selector of a mapped `PERSON` template, under the
`identifier_issuer` of the issuer's `[[auth.party]]` table. A `PERSON`
binds a professional only while it performs a live professional `ROLE`, and
a representative only while it is the source of a recorded representation
relationship. The identifier gets there in one of four ways:

1. **SCIM.** Your identity provider provisions the person through
   [FerroEHR's SCIM 2.0 endpoint](../installation/config-auth.md#authscim) at
   `/scim/v2/`, sending the key in
   `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier`.
   FerroEHR creates the `PERSON` and its professional `ROLE` in one
   contribution.
2. **Just-in-time creation** at the first login, off by default per issuer
   ([configuration](../installation/config-auth.md#just-in-time-creation)).
3. **The demographic API**, under the administrator role.
4. **An existing `PERSON`** that already carries the identifier, adopted as
   [the migration](../installation/party-binding-migration.md) describes.

In the `openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2` shape, the key is
one `DV_IDENTIFIER` in the cluster `PARTY.details` holds, with the
`[[auth.party]] identifier_issuer` as `issuer` and its `identifier_type` as
`type`:

```json
{
  "_type": "CLUSTER",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2",
  "name": { "_type": "DV_TEXT", "value": "Person identifier" },
  "items": [
    {
      "_type": "ELEMENT",
      "archetype_node_id": "at0001",
      "name": { "_type": "DV_TEXT", "value": "Identifier" },
      "value": {
        "_type": "DV_IDENTIFIER",
        "issuer": "https://login.microsoftonline.com/<tenant-id>",
        "id": "<the Entra object id>",
        "type": "guid"
      }
    }
  ]
}
```

### Who may set it

- A professional's `PERSON`, its professional `ROLE` and every provisioned
  `GROUP` are written only by the SCIM client that owns them or by an
  administrator. A professional editing their own `PERSON` cannot change
  their binding.
- The binding key never changes once set (RFC 7643 §2.2): a version that
  changes it is refused with `403`, and
  [`POST /admin/party/binding-key`](../operations-admin-apis.md) is the one
  change.
- The professional `ROLE` is created or retired only by a SCIM client,
  just-in-time creation under its admission rules, or an administrator.
- A representative's `PERSON` and its representation relationship are
  registered by an administrator or by the proxy service's registration,
  never by SCIM.

## The identity provider side

FerroEHR is an OAuth 2.0 resource server and never speaks SAML. The
walk-throughs, the generic standards path first, are on
[Enterprise identity providers](../identity-providers.md#the-generic-standards-path).
