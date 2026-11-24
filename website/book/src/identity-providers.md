# Enterprise identity providers

FerroEHR does not manage users. There is no user table, no user API, and no
plan to add one: **identity administration is delegated to your identity
provider (IdP)**, and the CDR consumes standard OAuth 2.0 bearer tokens. This page
records that posture and walks through connecting an identity provider:
[behind FerroSMART](#behind-ferrosmart) first, then [the generic standards
path](#the-generic-standards-path) for any authorization server whose access
tokens follow RFC 9068, then **Keycloak** trusted directly, and the identity
providers that run upstream of FerroSMART: **Microsoft Entra ID**, **AD
FS**, **Okta**, a SAML identity provider and plain-LDAP directories.

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
  its claims, and enforces [RBAC, the access evaluator and per-EHR access
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
2. **Role mining** (`[auth.issuer.<name>]`): `FERROEHR__AUTH__ISSUER__<NAME>__ROLE_CLAIMS`
   (default `["roles","groups","entitlements"]`, the RFC 9068 §2.2.3.1
   carriers; a Keycloak realm adds `realm_access.roles`) names the
   JWT claim paths whose values become the caller's roles for the
   [role layer](security.md#authorization). A path may be dotted to walk nested
   claims, and a claim holding a single string is accepted as readily as an
   array.
3. **SMART on openEHR scopes**: every route requires a scope from the
   token's `scope` claim (`user/composition-*.cruds`, `user/aql-*.s` and so
   on; see [SMART App Launch](smart-app-launch.md)). Behind FerroSMART,
   FerroSMART grants them per request ([Behind FerroSMART](#behind-ferrosmart));
   a provider FerroEHR trusts directly has to mint them itself. A token with
   roles and no SMART on openEHR scope authenticates and is then refused with
   `403` on every route.

Everything below is those three pointed at a different issuer.

## Behind FerroSMART

For SMART apps, FerroEHR trusts one issuer, **FerroSMART**, the FerroHEALTH
family's SMART App Launch 2.x authorization server, and your identity provider
runs upstream of FerroSMART for login only
([Behind FerroSMART](behind-ferrosmart.md)). FerroEHR never sees the provider's
tokens: FerroSMART logs the person in at the provider, applies its own policy
and consent, and issues the RFC 9068 access tokens FerroEHR reads from one
`[auth.issuer.<name>]` table naming its `issuer`.

FerroSMART supports these upstream providers, each with its setup in
FerroSMART's repository:

| Provider | FerroSMART's setup |
|---|---|
| Keycloak | the realm and configuration of [`examples/keycloak/`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/app/ferrosmart/examples/keycloak/), run by [`compose.keycloak.yaml`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/compose.keycloak.yaml) (architecture §13.16) |
| ZITADEL | the bootstrap of [`examples/zitadel/`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/app/ferrosmart/examples/zitadel/), run by [`compose.zitadel.yaml`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/compose.zitadel.yaml) (§13.20) |
| Microsoft Entra ID | [`examples/entra/ferrosmart.toml`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/app/ferrosmart/examples/entra/ferrosmart.toml) (§13.18) |
| Okta | [`examples/okta/ferrosmart.toml`](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/app/ferrosmart/examples/okta/ferrosmart.toml) (§13.19) |

FerroEHR's quickstart runs the first two upstream of FerroSMART
([Docker Compose](installation/compose.md#behind-ferrosmart)).

What changes against a provider FerroEHR trusts directly, such as the
[Keycloak](#keycloak) issuer below:

- **Scopes per request, with consent.** FerroSMART grants the intersection of
  what the app asks, the client's ceiling and the scopes the person's roles map
  to (`[[ferrosmart.role]]`), and the person approves or narrows them on its
  consent page. A directly trusted provider puts a fixed set in every token.
- **Launch context.** FerroSMART puts `ehrId` (at an openEHR base), `patient`
  and `fhirUser` in the token response and the access token, from the
  viewer's EHR launch or its own patient picker
  ([SMART App Launch](smart-app-launch.md)). A `patient/` grant reaches only
  the EHR its `ehrId` names, and it is trust in the grant and nothing more:
  whether a caller is the record's person comes from the party binding alone
  ([A person's own record](behind-ferrosmart.md#a-persons-own-record)).
- **One issuer for professionals and patients.** FerroSMART signs both, each
  named by their `fhirUser`: a patient signs in, picks their EHR and gets a
  `patient/` grant to it.
- **The person's key is FerroSMART's subject.** FerroSMART's `sub` is
  pairwise per provider: base64url of the SHA-256 of the provider's issuer,
  a zero octet and the provider's `sub`. A PERSON's SCIM `loginIdentifier` is
  that value, under the `[auth.issuer.<name>.party]` identifier issuer of FerroSMART's
  table. Moving people to another provider changes every key, and a
  deployment re-provisions their logins.
- **Roles.** FerroSMART writes the person's roles, as it mapped them from the
  provider, into the access token's `roles` claim (RFC 9068 §2.2.3.1) from
  FerroSMART 0.2.0, the release the quickstart pins
  (FerroHEALTH/FerroSMART#649). Name FerroSMART's table in `[authz.rbac]
  role_issuers` for them to grant `admin_role` or the emergency role.
- **The product scope.** A person reaches the ADMIN API only with the
  product scope, which FerroSMART grants through a role that maps to it from
  FerroSMART 0.2.0 (FerroHEALTH/FerroSMART#647).
- **Clients** register at FerroSMART, whose registry is the one list of
  clients. A system client registers for Backend Services, with a JWK Set,
  and FerroEHR grants each its operations in the CDR as for any system
  caller.

```toml
[server]
public_base_url = "https://ehr.example.org"

[auth.issuer.smart]
issuer = "https://smart.example.org"

[auth.issuer.smart.party]
identifier_issuer = "urn:example:ferrosmart-login"
identifier_type = "login"

[authz.rbac]
role_issuers = ["smart"]
```

The audience is the openEHR base FerroEHR derives from `public_base_url`,
the base FerroSMART names in `aud`.

## Who the caller is

`ferrohealth-smart` reads every token's caller from its standard claims, the
same way for every issuer (SMART App Launch 2.2.0, Scopes and Launch Context;
RFC 9068 §2.2; RFC 8693 §4.1), and the server acts on the kind:

- **A person** is a user whose token's `fhirUser` is a `Practitioner`, a
  `PractitionerRole`, a `Person`, or of no stated type, or a Basic user. The
  login, the token's `sub` under the issuer's party table, is bound to the
  `PERSON` carrying its key, and the person commits under that `PERSON`.
- **A patient** is a user whose `fhirUser` is the `Patient` their `patient`
  launch context names. Their login is bound like a person's, and they are
  the record's person only on the EHRs whose subject is their `PERSON`.
- **A representative** is a user whose `fhirUser` is a `RelatedPerson`, or
  whose `PERSON` is a representative's alone, admitted for a subject the CDR
  stores a live representation of
  ([Representatives](operational-policy.md#representatives)).
- **A delegated user** is a token whose RFC 8693 `act` names an actor, such
  as a service that exchanged the user's token or an administrator who
  impersonates them. The token's subject is the user, so the request is bound
  and committed exactly as the user's own; the actor does not change who
  committed. The access record carries `acting_mode = delegated`, the client
  and the actor chain (EHDS Annex II 3.2(b) asks for the natural person who
  accessed). An actor that names no issuer is recorded under the token's
  issuer.
- **A system caller** is a client credentials token, whose `sub` is its
  `client_id` (RFC 9068 §2.2). A Basic user is never one. A system caller
  acts for no person, so it holds `system/` scopes only, and a token with a
  `user/` or `patient/` scope is refused with `403`. It reads under its
  `system/` scopes. It commits into an EHR only as a client the CDR grants
  `commit_composition`: an `ACTOR` carrying its `client_id` and performing a
  `ROLE` whose capability grants it
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
    launch context;
  - it is an access-service caller by its kind, so an opted-out EHR answers
    `404` as an unknown one does (EHDS Art. 10(1)). Only an external person
    under an agreement with `opt_out_override` reaches one, under an emergency
    purpose, recorded as an override; an organisation never does;
  - a read under an agreement with a `consent` condition needs the consent
    decision reference for the patient (Wabvpz Art. 1(j) and Art. 15a(1)):
    the token carries it in the claim or `authorization_details` member its
    `[auth.issuer.<name>.federation]` table maps to `consent.reference`, as one string
    (a coded value refuses the token with `401`), and its `patient/` grant
    holds it to the launch-context patient. A read without it is refused with
    `403`, and the access record carries the reference with the other
    external fields;
  - the professional roles and the facility types its token attests, in the
    claims its `[auth.issuer.<name>.federation]` table maps to
    `professional.hp_professional_role` and `provider.facility_type`, are
    read as the caller's roles and its organisation's types, each a code of
    the system the value names. The controller's rules name callers by them
    ([Controller rules](operational-policy.md#controller-rules)), so a
    national rule such as one that keeps insurers or occupational physicians
    out of an exchange is a rule you commit, refused with `403` as
    `refused-policy-denied`. A value that names no system matches no rule;
  - a `chained` organisation request carries the requester's correlation id
    in the claim or attested `authorization_details` member its
    `[auth.issuer.<name>.federation]` table maps to `correlation_id`, as one string; a
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
    `[auth.issuer.<name>.federation]` table when the token does not carry it;
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

A `federation` issuer with `contact_point = true` in its `[auth.issuer.<name>.federation]`
table is a MyHealth@EU national contact point (NCPeH-A's authorization
server). Its person agreement names the service it serves in `ehdsi_service`
(`patient_summary`, `eprescription` or `orcd`), and its issuer goes into
sets `patient_context` in its `[auth.issuer.<name>.profile]` table. Every
request runs with a sender-constrained token
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
- The patient is resolved to its EHRs in the party domain, with no call to
  the identity source: the patient's `PERSON` carries the identifier at the
  `[authz.party_binding.patient] identifier` path, compared on the value and
  the assigning authority (the `DV_IDENTIFIER` issuer), and its EHRs are the
  ones its own reference and its `record_subject` references name
  ([The patient's EHRs](installation/config-integrations.md#the-patients-ehrs)).
  Under `record.subject_index = "local"` an EHR whose `EHR_STATUS` names the
  identifier itself is found too. The request must name one of those EHRs:
  the route's `{ehr_id}`, or a query's `ehr_id`. The EHR stands in for the
  launch context the token does not carry (eHDSI XCA Profile 8.0.1 §4.2.1).
  An identifier no `PERSON`, or more than one, carries finds no EHR.
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

FerroEHR trusts an authorization server whose access tokens follow RFC 9068:
`typ` `at+jwt`, and `iss`, `sub`, `aud`, `exp`, `iat`, `jti`, `client_id`
and `scope` in every token (RFC 9068 §2.2, SMART App Launch 2.2.0). Its
registry is the one list of clients: a token whose `aud` names FerroEHR
admits the client its `client_id` names, and FerroEHR keeps no client list of
its own. Configure three things:

1. **The issuer.** One table naming it:

   ```toml
   [auth.issuer.as]
   issuer = "https://as.example.com"
   audience = ["ferroehr"]
   ```

   A server that signs its tokens with `typ` `JWT` names it in
   `[auth.issuer.as.profile] token_types`.

2. **The binding, on `sub`.** OpenID Connect Core 1.0 §5.7 names `iss` with
   `sub` as the only stable identifier of an end-user, so a login is bound by
   its token's `sub`. The `[auth.issuer.<name>.party]` table names the
   identifier the key is matched against:

   ```toml
   [auth.issuer.as.party]
   identifier_issuer = "urn:example:staff-login"
   identifier_type = "uuid"
   ```

   An `email`, `phone_number`, `preferred_username` or `name` is never a key.
3. **The SCIM client.** Your authorization server issues the provisioning
   client an access token by the client credentials grant (RFC 6749 §4.4), or
   in exchange for a client assertion (RFC 7523 §2.2). Grant the client
   [`scim_provision`](installation/config-auth.md#authscim) in the CDR with
   the `binding_issuer` of step 2. The SCIM client sends the key as
   `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier`
   (RFC 7644).

A service that calls FerroEHR for a signed-in professional, such as
FerroBRIDGE, exchanges the professional's token at the authorization server
by RFC 8693 token exchange. The request is bound to the professional, and
the access record names the service.

[Who is the committer](security/committer.md) shows what the server records
for each of these callers.

## Keycloak

Keycloak 26.8 signs its access tokens with `typ` `JWT`, names the client in
`azp` and the scopes in `scope`, and carries no `client_id` unless a mapper
adds it. FerroEHR trusts a realm directly once every client's tokens carry
`client_id`:

1. **`client_id` in every token.** Add a *Hardcoded claim* mapper named
   `client_id` to each client, whose value is the client's own id, and add it
   to the access token. A client credentials token's `sub` is the service
   account's user id, so give each service account's tokens a `sub` equal to
   the client id with a second hardcoded mapper; FerroEHR reads a token whose
   `sub` is its `client_id` as a system caller (RFC 9068 §2.2).
2. **The token type.** Set `[auth.issuer.<name>.profile] token_types =
   ["JWT"]`.

   ```toml
   [auth.issuer.staff]
   issuer = "https://keycloak.example.com/realms/staff"
   audience = ["ferroehr"]

   [auth.issuer.staff.profile]
   token_types = ["JWT"]
   ```

3. **The binding on `sub`.** Keycloak's `sub` is the user id, which no user
   can change.
4. **The audience for an onward caller.** Keycloak's standard token exchange
   requires "the requester client set as an audience in the `aud` claim" of
   the subject token (*Securing applications*, token exchange). Add an
   audience mapper to the client scope your professionals' tokens carry that
   puts the bridge's or gateway's client in `aud`, and let the onward caller
   name FerroEHR's client id in the `audience` parameter.
5. **Impersonation.** Keycloak 26.8 puts the administrator who impersonates a
   user into an RFC 8693 `act` claim of every token of the impersonation
   session, which "is always present and cannot be disabled" (Keycloak 26.8.0
   release notes, *Track impersonation across token lifecycle and downstream
   services*). FerroEHR reads it as asserted: the request is bound to the
   impersonated user and the access record names the administrator as the
   actor. On a binding realm, check that no user or group holds the realm
   `impersonation` role or the fine-grained admin permission `impersonate`,
   and disable the legacy token exchange. A Keycloak older than 26.8 writes
   no `act`, so there an impersonation cannot be seen at all.
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
   creation.

## Identity providers upstream of FerroSMART

**Microsoft Entra ID** signs its access tokens with `typ` `JWT`, names the
scopes in `scp` and the client in `azp` (Microsoft identity platform,
*Access token claims reference*). **Okta** names the scopes in `scp` and the
client in `cid` (Okta, *OpenID Connect & OAuth 2.0 API*). **AD FS** issues
neither `client_id` nor `scope`. None of them issues the RFC 9068 tokens
FerroEHR reads, so each runs upstream of FerroSMART for login, as the table
in [Behind FerroSMART](#behind-ferrosmart) lists. The same holds for:

- **A SAML 2.0 identity provider.** FerroEHR never sees SAML. Keycloak or
  Entra ID brokers it by Web Browser SSO (OASIS SAML V2.0 Profiles §4.1), and
  FerroSMART logs the person in there. In Keycloak, set the identity
  provider's *Principal Type* to the Subject `NameID` with the persistent
  format (`urn:oasis:names:tc:SAML:2.0:nameid-format:persistent`) or to an
  immutable attribute, so the brokered user's `sub` stays the same from login
  to login.
- **A plain LDAP directory.** The CDR does not speak LDAP, by design: an LDAP
  bind would put password handling back inside the CDR. Front Active
  Directory with AD FS or Entra ID, and any other directory with Keycloak's
  LDAP user federation.

Never bind on `email`, `phone_number`, `preferred_username`, `name`, `upn` or
`unique_name`, whatever an assertion maps them from (OpenID Connect Core 1.0
§5.7).

> [!IMPORTANT]
> A role must arrive in a **role claim**, not in `scope`. The OAuth2 `scope`
> claim grants a client delegated authority
> ([RFC 6749 §3.3](https://www.rfc-editor.org/rfc/rfc6749#section-3.3)) and says
> nothing about the subject's roles, so it is not a role source.

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
- **Keycloak:** the built-in `profile` client scope
  maps the name claims; check that its mappers add them to the access token.
  Make the name attributes administrator-controlled as the key is: in the
  realm's user profile, set the attribute's *Who can edit* permission to
  *Admin* only (`"permissions": { "edit": ["admin"] }` in the JSON editor),
  which Keycloak defines as writable only from the administrative context
  (Server Administration Guide, *Managing user attributes*, Permission). A
  professional who could edit their own name could change what their first
  login writes.

- **FerroSMART:** its access token carries the launch context, `fhirUser`
  and the person's roles and no name claims, so just-in-time creation does not
  apply behind it: provision the people through SCIM, their `loginIdentifier`
  FerroSMART's subject ([Behind FerroSMART](#behind-ferrosmart)).

The `userName` comes from `user_name_claim`, or the key when it is unset, and
never from a claim the user can change. SCIM completes a provisional `PERSON`
later: a client whose `binding_issuer` is the issuer's `identifier_issuer`
finds it by `userName`, sets the `externalId`, and a complete `PUT` makes it
`complete`.

## Serving SMART apps

SMART apps take their tokens from FerroSMART, with your identity provider
upstream of it ([Behind FerroSMART](#behind-ferrosmart)). FerroEHR's discovery
document then describes FerroSMART, so an app that starts at FerroEHR's base
finds it, and the server it *advertises* to apps is the one it *accepts*
tokens from ([SMART App Launch](smart-app-launch.md#the-discovery-document)).
