# Enterprise identity providers

FerroEHR does not manage users. There is no user table, no user API, and no
plan to add one: **identity administration is delegated to your identity
provider (IdP)**, and the CDR consumes standard OAuth 2.0 bearer tokens. This page
records that posture and walks through connecting an identity provider:
[the generic standards path](#the-generic-standards-path) first, then
**Microsoft Entra ID** (Azure AD), **AD FS**, **Okta**, **Keycloak**, a SAML
identity provider behind one of them, and the answer for plain-LDAP
directories.

<!-- toc -->

## The posture: users live in the IdP

A clinical data repository is the wrong place to store credentials. A user
store would make the CDR an authentication product (password lifecycle,
lockout policy, MFA, recovery flows, and the largest new attack surface the
product could grow) duplicating what a dedicated IdP already does under your
existing governance. So the split is deliberate and permanent:

- **The IdP owns identities**: accounts, passwords, MFA, group/role
  membership, lifecycle (joiners/movers/leavers), and session policy.
- **The CDR owns authorization**: it validates the token, mines roles from
  its claims, and enforces [RBAC/ABAC and per-EHR access
  control](security.md#authorization) on every request.

The HTTP Basic user list in `ferroehr.toml` holds persons only, and it is not
a user store: each Basic user maps to the `PERSON` that carries its binding
key, which the identity provider provisions through SCIM
([bootstrap and migration](installation/party-binding-migration.md)). A
machine authenticates with OAuth 2.0 client credentials and is granted what
it may do in the CDR ([Operational policy](operational-policy.md#what-a-system-client-may-do)),
and most people authenticate with bearer tokens
from the IdP.

> [!NOTE]
> The viewer follows the same rule: it authenticates against the same
> credentials the CDR accepts (the same OIDC issuer, or Basic) and has no
> user-management screens. To create, disable, or re-role a user, use your IdP's
> own administration surface.

## How the CDR consumes an IdP

Three things do all the work:

1. **Token validation** (`[auth]`): the server reads the issuer's metadata
   and key set at boot and validates each bearer token's signature
   (asymmetric algorithms only), `iss`, `exp`/`nbf`, `aud` and `typ`; the
   audience list is mandatory, so a server with none refuses to boot rather
   than accepting another service's token. See the
   [bearer settings table](security.md#authentication).
2. **Role mining** (`[[auth.issuer]]`): `FERROEHR__AUTH__ISSUER__0__ROLE_CLAIMS`
   (default `["roles","groups","entitlements"]`, the RFC 9068 §2.2.3.1
   carriers; a Keycloak realm adds `realm_access.roles`) names the
   JWT claim paths whose values become the caller's roles for the
   [role layer](security.md#authorization). A path may be dotted to walk nested
   claims, and a claim holding a single string is accepted as readily as an
   array.
3. **SMART on openEHR scopes**: every route requires a scope from the
   token's `scope` claim (`user/composition-*.cruds`, `user/aql-*.s` and so
   on; see [SMART App Launch](smart-app-launch.md)). Your IdP has to mint
   them, for example as Keycloak client scopes assigned to the client. A token
   with roles and no SMART on openEHR scope authenticates and is then refused
   with `403` on every route.

Everything below is those three pointed at a different issuer.

## Who the caller is

`ferrohealth-smart` classifies every token by its issuer's `kind` and
profile, and the server acts on the kind:

- **A person** signs in at a `persons` issuer through a first-party client,
  or with HTTP Basic. The login is bound to the `PERSON` carrying its key,
  and the person commits under that `PERSON`.
- **A delegated person** reaches the server through a client or an RFC 8693
  actor that the issuer's profile lists in `delegates`. The token's subject is
  the person, so the request is bound and committed exactly as the person's
  own; the client and the actor do not change who committed. The access
  record carries `acting_mode = delegated`, the client and the actor chain.
- **A Keycloak impersonation** is a delegated person. Keycloak 26.8 puts the
  administrator who impersonates a user into an RFC 8693 `act` claim of every
  token of the impersonation session, `{"sub": <the administrator's user
  id>, "preferred_username": …}`, which "is always present and cannot be
  disabled" (Keycloak 26.8.0 release notes, *Track impersonation across
  token lifecycle and downstream services*). An impersonation is therefore
  refused like any undeclared actor, unless the issuer's `delegates` lists
  the administrator as an actor (`{ sub = "<user id>" }`). Then the request is
  bound to the impersonated user and commits as that user, and the access
  record names the administrator as the actor (EHDS Annex II 3.2(b) asks
  for the natural person who accessed). Keycloak's `act` names no issuer,
  so the actor is recorded under the token's issuer. The `impersonator`
  claim (`id`, `username`) of Keycloak's built-in IMPERSONATOR_ID and
  IMPERSONATOR_USERNAME session note mappers repeats `act` and decides
  nothing. To refuse every impersonation, add those mappers and set
  `impersonation_marker = "impersonator"` on the issuer: a token carrying
  the claim is refused before any actor is read. A Keycloak older than 26.8
  writes no `act`, so there the marker is the only way to see an
  impersonation at all.
- **A guest**, by the profile's `guest` markers (Entra ID marks one with
  `acct = 1` or an `idp` other than the issuer), is refused with `403`
  (`refused-guest`) unless the issuer's `[[auth.party]]` table sets
  `guests = true`. Admit guests only for people acting under your own
  responsibility (GDPR Art. 9(3) and Art. 29), such as contractors.
- **A system caller** is an app-only token by the profile's `app_only`
  markers: `sub` equal to the client claim under `rfc9068`, `idtyp = app`
  without `scp` under `entra`, and the listed service-account subjects under
  `keycloak`. A Basic user is never one. A system caller acts for no person, so
  it holds `system/` scopes only, and a token with a `user/` or `patient/` scope
  is refused with `403`. It reads under its `system/` scopes. It commits into an
  EHR only as a client the CDR grants `commit_composition`: an `ACTOR`
  carrying its `client_id` and performing a `ROLE`
  whose capability grants it
  ([Operational policy](operational-policy.md#what-a-system-client-may-do)),
  under the committer `PARTY_IDENTIFIED{name = <client>, identifiers =
  [{issuer = <iss>, id = <client>, type = "client"}]}` with an `external_ref`
  to that `ACTOR`. The grant decides the templates it writes, the facilities
  it writes at and whether it relays a person's act or registers a device's;
  an import or an export needs a grant of its own.
- **A verified external person or organisation** of a `federation` issuer is
  admitted only under the federation agreement its partner holds in the CDR
  for its accessor kind, the partner being the party that carries the
  token's issuer and `client_id` as a `client_id` identifier
  ([Operational policy](operational-policy.md#federation-agreements)), and
  refused with `403` without one or under the other accessor's.
  Its rights come from the agreement, never from a role:
  - it holds only the `patient/` scopes the agreement `reads` (beside
    `openid`, `profile`, `fhirUser`, `online_access` and `launch`), so a token
    with any other scope is refused, and it reaches only the EHR of its
    launch context, which needs the issuer in `[auth.smart] patient_issuers`;
  - it is an access-service caller by its kind, so an opted-out EHR answers
    `404` as an unknown one does (EHDS Art. 10(1)). Only an external person
    under an agreement with `opt_out_override` reaches one, under an emergency
    purpose, recorded as an override; an organisation never does;
  - a read under an agreement with a `consent` condition needs the consent
    decision reference for the patient (Wabvpz Art. 1(j) and Art. 15a(1)):
    the token carries it in the claim or `authorization_details` member its
    `[[auth.federation]]` table maps to `consent.reference`, as one string
    (a coded value refuses the token with `401`), and its `patient/` grant
    holds it to the launch-context patient. A read without it is refused with
    `403`, and the access record carries the reference with the other
    external fields;
  - under an NL agreement, Wabvpz Art. 15f holds an insurer and an
    occupational or insurance physician out. A caller whose `nl.role` is the
    UZI role 01.008 (bedrijfsarts) or 01.048 (verzekeringsarts) is refused
    with `403` as `refused-wabvpz-role`, and a caller whose
    `nl.organisation_type` is missing or is not a care provider type of
    Nictiz's `OrganisatieTypeCodelijst` is refused with `403` as
    `refused-wabvpz-organisation-type`, which is how an insurer is refused;
  - a `chained` organisation request carries the requester's correlation id
    in the claim or attested `authorization_details` member its
    `[[auth.federation]]` table maps to `correlation_id`, as one string; a
    request without it is refused with `403`, and a request header never
    stands in for it;
  - a caller above its agreement's `caller_rate` is answered `429` with
    `Retry-After` before any patient lookup, a MyHealth@EU contact point's
    included;
  - an external person commits only a COMPOSITION of a template the
    agreement's `writes` lists; an organisation commits nothing, and neither
    attests or writes a demographic party. Neither deletes, except the
    discard of a dispensation described under
    [the MyHealth@EU contact-point route](#the-myhealtheu-contact-point-route);
  - an external person's commit records the server's committer, a
    `PARTY_IDENTIFIED` with no `external_ref`: the asserted name as one
    string, the professional's `hp_identifier` under the issuer
    `"<country_code>:<issuing_authority_name>"` with type `hp_identifier`,
    and the token's `iss` and `sub` with type `oauth2`. The country must be
    an upper-case ISO 3166-1 alpha-2 code, and the issuing authority is
    NFC-normalised with its white space trimmed and collapsed and may hold no
    `:`, control or bidi character. A missing or failing field is refused
    with `403` naming it. Map the authority with a `constant` source in the
    `[[auth.federation]]` table when the token does not carry it;
  - the COMPOSITION names the healthcare provider the token asserts in
    `EVENT_CONTEXT.health_care_facility`, or with no context in
    `feeder_audit.originating_system_audit.location`: a `DV_IDENTIFIER` with
    the `provider.healthcare_provider_identifier` as `id` and the
    `provider.issuing_authority_name` as `issuer`. The server writes no
    facility and refuses a composition without it with `422` (EHDS
    Art. 13(4)). This is the domestic form; the MyHealth@EU contact-point
    form follows.
- **Attestations** come only from a person bound to a professional `PERSON`.
  A patient, a representative, a system caller and an unbound caller are
  refused with `403` (RM common master04 §Attestation).

### The MyHealth@EU contact-point route

A `federation` issuer with `contact_point = true` in its `[[auth.federation]]`
table is a MyHealth@EU national contact point (NCPeH-A's authorization
server). Its person agreement names the service it serves in `ehdsi_service`
(`patient_summary`, `eprescription` or `orcd`), and its issuer goes into
`[auth.smart] patient_issuers`. Every request runs with a sender-constrained token
(`sender_constraint = "required"`). On every patient-data route:

- The token carries the patient of the treatment relationship confirmation
  in `ehdsi.patient.id`, as `system|value`, the patient identifier's
  assigning authority and value, the form every family resource server
  reads. A token without it, or with a value of another form (an empty
  system or one with whitespace, or a value holding a control character,
  `|`, `^`, `&`, `~` or a backslash), is refused with `403` whose `detail` opens with
  `ERROR_SEC_DATA_INTEGRITY_NOT_ENSURED` (MyHealth@EU Requirements Catalogue
  10.0.0 requirement 11.01.02), recorded as `refused-trc-missing`, before
  the EHR index is asked.
- The patient is resolved to its EHR through the deployment's
  [EHR index](installation/config-integrations.md#pixm), compared on the value and the
  assigning authority, and the request must name that EHR: the route's
  `{ehr_id}`, or a query's `ehr_id`. The EHR stands in for the launch
  context the token does not carry (eHDSI XCA Profile 8.0.1 §4.2.1).
- Another patient's EHR, an EHR the server does not hold, an EHR an
  opted-out patient's record keeps out of reach, a TRC patient no EHR
  belongs to, and a request that names no EHR all get one answer, `404` of
  type [`trc-patient-mismatch`](using-the-api/problems.md#trc-patient-mismatch)
  whose `detail` opens with the service's `*_NOT_FOUND` code
  (`ERROR_PS_NOT_FOUND`, `ERROR_EP_NOT_FOUND` or `ERROR_ORCD_NOT_FOUND`),
  which the contact point answers as "No Data". Each is recorded as a
  `refused-trc-patient-mismatch` security event, once per request and never
  summarised (eHDSI XCA Profile 8.0.1 §4.2.2).
- Where the agreement sets `discard`, the person may discard a dispensation
  of a template its `writes` lists, as a logical delete
  (`DELETE /ehr/{ehr_id}/composition/{preceding_version_uid}`), within
  `discard_window` of its first commit (MyHealth@EU Requirements
  Catalogue 10.0.0, requirement 07.04). The eHDSI point of care is a name
  only, so the dispensation's `EVENT_CONTEXT.health_care_facility` must
  carry every identifier the discarding token names its point of care by:
  the `provider.healthcare_provider_identifier` under the
  `provider.issuing_authority_name`, and the same identifier under the issuer
  `urn:oasis:names:tc:xspa:1.0:subject:organization-id`, the HCPO's
  `organization-id` (eHDSI SAML Profile 10.0.0 §2.3). Any other discard is
  refused with `403`.
- The committer of a contact point's caller names the professional by the HP
  Identity Assertion's `NameID`: a `DV_IDENTIFIER` with the assertion's issuer
  (`ehdsi.assertion.issuer`, NCP-B) as `issuer`, the `NameID`
  (`ehdsi.professional.hp_identifier`) as `id`, and
  `hp_identifier:<NameID format>` as `type`, such as
  `hp_identifier:urn:oasis:names:tc:SAML:1.1:nameid-format:X509SubjectName`,
  because one string under two formats names two identifiers (eHDSI SAML
  Profile 10.0.0 §2.1). The token's `iss` and `sub` follow with type
  `oauth2`, as on a domestic route. A discard is a logical delete under the
  same committer.
- A contact point's COMPOSITION carries, in
  `EVENT_CONTEXT.health_care_facility.identifiers`, the provider's identifier
  under `provider.issuing_authority_name` and, as a second `DV_IDENTIFIER`,
  the HCPO's `organization-id` under the issuer
  `urn:oasis:names:tc:xspa:1.0:subject:organization-id`. A commit without the
  second is refused with `422`; the server writes neither.
- A token that carries permissions (`ehdsi.professional.permissions`) records
  a dispensation and discards one only with `PPD-046` among them, as the code
  or its `urn:oasis:names:tc:xspa:1.0:subject:hl7:permission:` URN (eHDSI SAML
  Profile 10.0.0 §2.5); otherwise it is refused with `403`.

## The generic standards path

Every walk-through on this page configures the same three things: an issuer
of kind `persons` that signs your professionals' access tokens, the
`[[auth.party]]` table their logins are bound through, and a SCIM client that
provisions their `PERSON`s. Configure them first against a standards-only
authorization server; the vendor sections after this one are the same
configuration through a preset (`entra`, `keycloak`, `okta`), each listing
the preset's deviations with their sources. Every preset key can be set on
the issuer table, which wins over the preset.

1. **The issuer.** The `rfc9068` profile is the default: an RFC 9068 access
   token with `typ` `at+jwt`, the client in `client_id` and the scopes in
   `scope`.

   ```toml
   [[auth.issuer]]
   issuer = "https://as.example.com"
   kind = "persons"
   audience = ["ferroehr"]
   first_party_clients = ["ferroehr-viewer"]
   ```

2. **The binding, on `sub`.** OpenID Connect Core 1.0 §5.7 names `iss` with
   `sub` as the only stable identifier of an end-user, so the `rfc9068`
   profile binds on `sub`. The `[[auth.party]]` table names the identifier
   the key is matched against:

   ```toml
   [[auth.party]]
   issuer = "https://as.example.com"
   identifier_issuer = "urn:example:staff-login"
   identifier_type = "uuid"
   ```

   An `email`, `phone_number`, `preferred_username` or `name` is never a key.
3. **The SCIM client.** Your authorization server issues the provisioning
   client an access token by the client credentials grant (RFC 6749 §4.4), or
   in exchange for a client assertion (RFC 7523 §2.2). Declare the
   authorization server as an issuer of kind `machines`, and grant the client
   [`scim_provision`](installation/config-auth.md#authscim) in the CDR with
   the `binding_issuer` of step 2. The SCIM client sends the key as
   `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier`
   (RFC 7644).
4. **An onward caller.** A service that calls FerroEHR for a signed-in
   professional, such as FerroBRIDGE, exchanges the professional's token at
   the authorization server by RFC 8693 token exchange. List the service in
   the issuer's `delegates` (`{ client = "<its client id>" }`, or `{ sub }` for
   an RFC 8693 actor): the request is bound to the professional, and the
   access record names the service.

[Who is the committer](security/committer.md) shows what the server records
for each of these callers.

## Microsoft Entra ID (Azure AD)

Entra ID exposes a standards-compliant OIDC issuer per tenant.

1. **Register an application** (Entra admin center → App registrations).
   Note the *Directory (tenant) ID* and the *Application (client) ID*.
2. **Define app roles** (App registration → App roles): create roles named
   after the CDR roles you use (for example `USER`, `CLINICAL`, `ADMIN`) and
   assign users/groups to them (Enterprise applications → your app → Users
   and groups). Entra puts assigned app roles in the token's `roles` claim.
3. **Expose an audience**: either use the client ID as the audience or add an
   *Application ID URI* (for example `api://ferroehr`).
4. **Point the CDR at the tenant issuer** and mine the `roles` claim:

   ```bash
   export FERROEHR__AUTH__ISSUER__0__ISSUER=https://login.microsoftonline.com/<tenant-id>/v2.0
   export FERROEHR__AUTH__ISSUER__0__KIND=persons
   # Entra ID mints no RFC 9068 tokens (`typ` JWT, the scopes in `scp`), which
   # the `entra` profile reads.
   export FERROEHR__AUTH__ISSUER__0__PROFILE=entra
   export FERROEHR__AUTH__ISSUER__0__AUDIENCE=api://ferroehr
   export FERROEHR__AUTH__ISSUER__0__FIRST_PARTY_CLIENTS=<the client id of your app>
   export FERROEHR__AUTH__ISSUER__0__ROLE_CLAIMS=roles
   # Its roles may grant the administrator role.
   export FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS=https://login.microsoftonline.com/<tenant-id>/v2.0
   ```

5. **Verify**: request a token for the app (any OAuth2 client credential or
   auth-code flow) and call the API. Read the status codes as a diagnostic:

   | Status | What it tells you |
   |---|---|
   | `200`-family | roles arrived and the operation was permitted |
   | `401` | no credential, or one the server rejected (the body deliberately never says which) |
   | `403` | the token is valid but carries no role or SMART on openEHR scope the operation needs: a clinical call needs a matching scope and, unless every SMART on openEHR scope it holds is a `system/` scope, at least one role, an admin call needs the admin role and the product scope |
   | `400` | the `Authorization` header itself is malformed; no credential was ever read |
   | `503` | the CDR could not read your issuer's key set again, so the token was never judged: check network egress, not the token |

> [!IMPORTANT]
> A role must arrive in a **role claim**, not in `scope`. The OAuth2 `scope`
> claim grants a client delegated authority
> ([RFC 6749 §3.3](https://www.rfc-editor.org/rfc/rfc6749#section-3.3)) and says
> nothing about the subject's roles, so it is not a role source. If your IdP is
> configured to put role names in `scope`, map them into `roles` (or another
> entry of `ROLE_CLAIMS`) instead.

> [!TIP]
> Group-based deployments can emit the `groups` claim instead and list it in
> `ROLE_CLAIMS`, but group claims arrive as object IDs unless you configure
> group names, so app roles usually read better in policy.

> [!WARNING]
> Two configuration mistakes are boot errors rather than runtime surprises, so
> you will find them the first time you start the server: an issuer that is not
> an `https` URL with no query or fragment, and an empty audience list. Both are
> deliberate: the second is what stops this server accepting a token minted for
> a different service. An issuer the server cannot reach at boot refuses the
> start too.

### Binding and provisioning with the `entra` preset

The `entra` preset is the generic path with Entra ID's deviations, each from
Microsoft's *Access token claims reference* (claims) or *Optional claims
reference* (optional):

| Preset key | Value | Source |
|---|---|---|
| `person_key` | `oid` | claims, `oid`: "The immutable identifier for the requestor"; `sub` is a pairwise value per application and never a key |
| `tenant_claim` | `tid` | claims, `tid` |
| `token_types` | `["JWT"]` | claims, header `typ` "always `JWT`" |
| `scope_claim`, `app_only_scope_claim` | `scp` (string), `roles` (array) | claims, `scp` and `roles` |
| `client_claims` | `["azp"]` | claims, `azp` |
| `app_only` | `idtyp = "app"`, and no `scp` | optional, `idtyp`; claims, `scp` "Only included for user tokens" |
| `guest` | an `idp` other than the issuer, or `acct = 1` | claims, `idp`; optional, `acct` |
| `expect` | `ver = "2.0"` | claims, `ver` |
| `user_claims` | `["acct"]` | optional, `acct`, emitted only when the registration asks for it |
| `unstable_claims` | `upn`, `unique_name` | claims, `unique_name`; optional, `upn` |

Binding on `oid` is FerroEHR's choice on Microsoft's "immutable identifier"
wording. For it to work, the app registration of FerroEHR's API, and of
every service such as FerroBRIDGE that calls it, needs:

- the optional claims `acct` (the guest marker: the preset requires it on
  every user token, so a registration without it is refused at the first
  token, naming the claim), `idtyp` (the app-only marker), `given_name` and
  `family_name`, and the `profile` scope that `oid` needs;
- `"requestedAccessTokenVersion": 2` in the manifest's `api` attribute
  (*Understand the app manifest*): FerroEHR accepts v2.0 tokens only, and a
  null value defaults to version 1.

Cross-tenant synchronisation and converted guests: a member-type user of
another tenant carries `acct = 0`, but an `idp` naming their home tenant. The
preset treats them as a guest by `idp`, so they bind only where the
`[[auth.party]]` table sets `guests = true`, and never as staff.

A person in more groups than fit in a token (the groups overage: "150 for
SAML tokens, 200 for JWT tokens", claims) carries no `groups` claim, and the
login is refused with `403` (`refused-groups-unavailable`) unless the issuer
sets `adapter = "idp-entra"` with an `[auth.idp.entra]` table, which reads the
groups from Microsoft Graph. Provision only the groups assigned to the
application to stay under the limit.

**Provisioning through SCIM.** The non-gallery enterprise application, its
Tenant URL `https://<host>/scim/v2/` and the steps on FerroEHR's side are in
[Provision from Microsoft Entra ID](installation/config-auth.md#provision-from-microsoft-entra-id).
On the Entra side:

- **Authentication.** Use the OAuth 2.0 client credentials grant against your
  own authorization server, or Workload Identity Federation, in which Entra
  presents a signed client assertion your authorization server exchanges.
  Never leave the Secret Token blank: Entra then sends its own test token,
  which every non-gallery application of the tenant receives.
- **Attribute mappings.** Replace the tutorial's `mailNickname` to
  `externalId` with `objectId` to `externalId`, because a mail nickname "may
  not be unique in the directory" (*Develop a SCIM endpoint*). Map `objectId`
  to `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier`
  as well, and `userPrincipalName` to `userName`.
- **Matching precedence.** Set `externalId` first and `userName` second.
- **Remove every other attribute** the FerroEHR mapping does not carry:
  "Schema discovery isn't currently supported on custom non-gallery SCIM
  application" (*Develop a SCIM endpoint*), so Entra cannot learn FerroEHR's
  schema and sends what its defaults name.

## AD FS (on-premises Active Directory)

AD FS 2016+ speaks OIDC. This is also the supported path for on-premises
Active Directory in general: front AD with AD FS (or another OIDC-capable
broker) rather than pointing anything at LDAP.

1. **Create an Application Group** (AD FS Management → Application Groups →
   *Web API* template, or *Server application + Web API* for interactive
   clients). The Web API's *identifier* becomes the token audience.
2. **Issue role claims**: on the Web API's *Issuance Transform Rules*, add a
   rule mapping AD group membership to the `role` claim (template: *Send
   Group Membership as a Claim*), one rule per CDR role.
3. **Point the CDR at the AD FS issuer** and mine the `role` claim:

   ```bash
   export FERROEHR__AUTH__ISSUER__0__ISSUER=https://adfs.example.com/adfs
   export FERROEHR__AUTH__ISSUER__0__KIND=persons
   export FERROEHR__AUTH__ISSUER__0__AUDIENCE=ferroehr-api
   # AD FS mints no RFC 9068 tokens: name the type and the claims its tokens carry.
   export FERROEHR__AUTH__ISSUER__0__TOKEN_TYPES=JWT
   export FERROEHR__AUTH__ISSUER__0__REQUIRED_CLAIMS=iss,sub,aud,exp
   export FERROEHR__AUTH__ISSUER__0__FIRST_PARTY_CLIENTS=<the client id of your app>
   export FERROEHR__AUTH__ISSUER__0__ROLE_CLAIMS=role
   export FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS=https://adfs.example.com/adfs
   ```

   Discovery works out of the box (`https://adfs.example.com/adfs/.well-known/openid-configuration`).
4. **Verify** with the same status table as above: `401` without a token, `403`
   with a token that lacks the required role.

> [!NOTE]
> AD FS emits a single string for one role and an array for several; the
> role-mining layer accepts both shapes on any configured claim path.

## "We only have LDAP"

The CDR does not speak LDAP, by design: LDAP bind would put password
handling back inside the CDR. Front the directory with an OIDC-capable
broker and connect that instead:

- **Active Directory** → AD FS (above) or Entra ID (if synced).
- **Generic LDAP** → Keycloak with LDAP user federation (the
  [Keycloak example](security.md#authentication) then applies verbatim), or
  any other OIDC provider that can federate LDAP.

The broker owns the LDAP bind; the CDR sees only signed tokens.

## Okta

The `okta` preset reads Okta's access tokens (Okta, *OpenID Connect & OAuth
2.0 API*, the reserved claims of the access token):

| Preset key | Value | Source |
|---|---|---|
| `person_key` | `uid` | `uid`: "A unique identifier for the user" |
| `client_claims` | `["cid"]` | `cid`: "Client ID of the client that requested the access token" |
| `scope_claim` | `scp` (array) | `scp`: "Array of scopes that are granted to this access token" |
| `app_only` | no `uid` | `uid` "isn't included in the access token if there is no user bound to it" |
| `token_types` | none | the page names no `typ`, so an `okta` issuer fails to load until you set `token_types` to what your Okta server writes |

```toml
[[auth.issuer]]
issuer = "https://example.okta.com/oauth2/default"
kind = "persons"
profile = "okta"
token_types = ["<the typ header your server writes>"]
audience = ["api://ferroehr"]
first_party_clients = ["<the viewer's client id>"]
```

Read the `typ` header of one of your server's access tokens and set it in
`token_types`; the preset deliberately names none.

## Keycloak

The `keycloak` preset reads Keycloak 26.8 access tokens: `azp` as the client,
`scope` as a string, `typ` `JWT` (confirm it against your own server), and no
app-only marker until you list your service accounts. Configure a realm for
binding as follows.

1. **Two realms.** An issuer has one `kind`, so professionals sign in at one
   realm (`kind = "persons"`) and patients at another (`kind = "patients"`).
2. **The binding on `sub`.** Keycloak's `sub` is the user id, which no user
   can change. A user-profile attribute can be the key instead only when its
   *Who can edit* permission is *Admin* (`"permissions": { "edit": ["admin"] }`,
   Server Administration Guide, *Managing user attributes*).
3. **Service accounts.** "A client's service account is a user account with
   username `service-account-CLIENT_ID`", so its `sub` is a user id. List
   every service account of the realm by its `sub` in the issuer's
   `app_only`; the empty list refuses the issuer at load. On a realm with
   just-in-time creation an unlisted service account would be read as a
   person, so the list must be complete.

   ```toml
   [[auth.issuer]]
   issuer = "https://keycloak.example.com/realms/staff"
   kind = "persons"
   profile = "keycloak"
   audience = ["ferroehr"]
   first_party_clients = ["ferroehr-viewer"]
   app_only = [{ subjects = ["<the sub of service-account-lab>"] }]
   ```

4. **The audience for an onward caller.** Keycloak's standard token exchange
   requires "the requester client set as an audience in the `aud` claim" of
   the subject token (*Securing applications*, token exchange). Add an
   audience mapper to the client scope your professionals' tokens carry that
   puts the bridge's or gateway's client in `aud`, and let the onward caller
   name FerroEHR's client id in the `audience` parameter. List the onward
   client in `delegates`.
5. **No impersonation on a binding realm.** "Any user with the
   `impersonation` role in the realm can impersonate a user" (*Impersonating
   users*). Check that no user or group holds the realm `impersonation` role,
   or the fine-grained admin permission `impersonate`, and that no client
   holds the legacy token exchange's impersonation permission; disable the
   legacy token exchange. Where an administrator must keep impersonation, add
   the built-in user session note mappers IMPERSONATOR_ID and
   IMPERSONATOR_USERNAME to the inbound client scope and set
   `impersonation_marker = "impersonator"`, so every impersonation is
   refused. Whether Keycloak 26.8 adds those mappers by default is not
   verified here; check the client scope. Keycloak 26.8 also writes an RFC
   8693 `act` claim into every impersonation token, so an impersonator your
   `delegates` does not list is refused even without the marker
   ([Who the caller is](#who-the-caller-is)).
6. **Realm hardening.** On a binding or just-in-time realm: no
   self-registration, no social or other brokered identity provider without
   an explicit allow-list, and no automatic account linking (the first login
   flow's "automatically set existing user", or linking by email). Each of
   them would let a person choose which account, and so which `PERSON`, a
   login reaches.
7. **SCIM.** Keycloak 26.8 promotes its SCIM API to supported, as a service
   provider for its own realm's users and groups (release notes, *SCIM API
   (supported)*). It provisions nothing outbound, so it cannot create
   FerroEHR's `PERSON`s or `GROUP`s: a deployment needs an upstream SCIM
   client, such as its identity governance or HR system, or just-in-time
   creation. Enabling Keycloak's own SCIM adds a second SCIM endpoint to the
   deployment, which FerroEHR does not read.

## A SAML identity provider through Keycloak or Entra ID

FerroEHR never sees SAML. It is an OAuth 2.0 resource server and validates
only access tokens, so a SAML 2.0 identity provider reaches it through an
authorization server that brokers it:

- **Keycloak** brokers it by Web Browser SSO (OASIS SAML V2.0 Profiles §4.1)
  as the service provider, and issues its own OpenID Connect tokens after the
  login. Set the identity provider's *Principal Type* to the Subject `NameID`
  with the persistent format (`urn:oasis:names:tc:SAML:2.0:nameid-format:persistent`,
  Keycloak's default *NameID Policy Format*) or to an immutable attribute, so
  the brokered user's Keycloak `sub` stays the same from login to login.
  Keycloak refuses a Subject `NameID` principal with the transient format.
  Map the name attributes to `given_name`, `family_name` and `name` for
  just-in-time creation, and the group attribute to the group claim.
- **Entra ID** federates the SAML identity provider for its tenant, and the
  `entra` preset reads the tokens it issues as above.

Never bind on `email`, `phone_number`, `preferred_username`, `name`, `upn` or
`unique_name`, whatever the SAML assertion maps them from (OpenID Connect
Core 1.0 §5.7).

## Just-in-time creation

An issuer can let a professional with no party work at their first login:
FerroEHR then creates a provisional `PERSON` from the access token and binds
the login to it, as
[the configuration reference](installation/config-auth.md#just-in-time-creation)
describes. It is off until an administrator writes a `create_at_login`
grant for the issuer in the CDR, whose admission conditions read the
claims below
([Operational policy](operational-policy.md#just-in-time-admission)). What
each identity provider must do for it:

- **Put the name claims in the access token.** FerroEHR reads the person's
  data from the access token only (RFC 9068 §2.2.2 lets an authorization
  server include resource owner attributes there), never from an ID token or
  the userinfo endpoint. A generic issuer emits the OpenID Connect Core 1.0
  §5.1 claims your mapped attributes name (`name`, `given_name`,
  `family_name`).
- **Microsoft Entra ID (the `entra` preset):** add the optional claims
  `given_name` and `family_name` to the access token of your app
  registration and request the `profile` scope: Entra documents both as v2.0
  optional claims that "Requires the `profile` scope" (Microsoft identity
  platform, *Optional claims reference*). Admit logins by group (an
  `admit_group` element of the `create_at_login` grant, holding an object
  id of `groups`) or by the `idp` claim (an `admit_idp` element), which
  "records the identity provider that authenticated the subject of the
  token" and differs from the issuer for a guest (*Access token claims
  reference*). The key is the object id `oid`, never `upn` or `email`, which
  Entra calls mutable.
- **Keycloak (the `keycloak` preset):** the built-in `profile` client scope
  maps the name claims; check that its mappers add them to the access token.
  Make the name attributes administrator-controlled as the key is: in the
  realm's user profile, set the attribute's *Who can edit* permission to
  *Admin* only (`"permissions": { "edit": ["admin"] }` in the JSON editor),
  which Keycloak defines as writable only from the administrative context
  (Server Administration Guide, *Managing user attributes*, Permission). A
  professional who could edit their own name could change what their first
  login writes.

The `userName` comes from `user_name_claim`, or the key when it is unset, and
never from a claim the user can change. SCIM completes a provisional `PERSON`
later: a client whose `binding_issuer` is the issuer's `identifier_issuer`
finds it by `userName`, sets the `externalId`, and a complete `PUT` makes it
`complete`.

## Serving SMART apps

If your IdP is also the authorization server for SMART App Launch apps, the same
`[auth]` issuer does double duty: the CDR validates the tokens those apps come
back with, and the discovery document it serves is built from that issuer's
metadata, so the server it *advertises* to apps is the one it *accepts* tokens
from. See [SMART App Launch](smart-app-launch.md).
