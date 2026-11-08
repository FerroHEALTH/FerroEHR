# Enterprise identity providers

FerroEHR does not manage users. There is no user table, no user API, and no
plan to add one: **identity administration is delegated to your identity
provider (IdP)**, and the CDR consumes standard OAuth 2.0 bearer tokens. This page
records that posture and walks through connecting the two enterprise IdPs we
are asked about most, **Microsoft Entra ID** (Azure AD) and **AD FS**, plus
the answer for plain-LDAP directories.

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

The HTTP Basic user list in `ferroehr.toml` is for machines and bootstrap,
not a user store; people authenticate with bearer tokens from the IdP.

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
  own; the client and the actor do not change who committed.
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
  EHR only as a client an `[[authz.system_client]]` table declares
  ([configuration](installation/config-auth.md#authzimport-and-authzsystem_client)),
  under the committer `PARTY_IDENTIFIED{name = <client>, identifiers =
  [{issuer = <iss>, id = <client>, type = "client"}]}` with no `external_ref`.
  - Each COMPOSITION it commits is of a template its `writes` lists, or the
    commit is refused with `403`.
  - It names a facility of its `facilities` in
    `context.health_care_facility.identifiers`, or, for a COMPOSITION with no
    context, in `feeder_audit.originating_system_audit.location.identifiers`.
    Otherwise it is refused with `422` naming the element; the server fills
    neither.
  - A device act names the device in `composer`, with an identifier of one of
    the client's `device_identifier_issuers` that is also
    `feeder_audit.originating_system_audit.system_id`, and is accepted only
    from a `device` client. Every other COMPOSITION records a person's act: it
    is accepted only from a `relay` client (`403` otherwise) and names the
    professional by an identifier in `composer` or in the `performer` of a
    context participation (`422` otherwise). An organisation `composer` is
    refused with `422`.
- **A verified external person or organisation** of a `federation` issuer is
  admitted only under the `[[authz.federation.agreement]]` of its issuer and
  accessor, and refused with `403` without one or under the other accessor's.
  Its rights come from the agreement, never from a role:
  - it holds only the `patient/` scopes the agreement `reads` (beside
    `openid`, `profile`, `fhirUser`, `online_access` and `launch`), so a token
    with any other scope is refused, and it reaches only the EHR of its
    launch context, which needs the issuer in `[smart] patient_issuers`;
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
  - an external person commits only a COMPOSITION of a template the
    agreement's `writes` lists; an organisation commits nothing, and neither
    deletes, attests or writes a demographic party;
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
    form, with the identity assertion's `NameID` and the `organization-id`,
    is planned.
- **Attestations** come only from a person bound to a professional `PERSON`.
  A patient, a representative, a system caller and an unbound caller are
  refused with `403` (RM common master04 §Attestation).

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

## Serving SMART apps

If your IdP is also the authorization server for SMART App Launch apps, the same
`[auth]` issuer does double duty: the CDR validates the tokens those apps come
back with, and the discovery document it serves is built from that issuer's
metadata, so the server it *advertises* to apps is the one it *accepts* tokens
from. See [SMART App Launch](smart-app-launch.md).
