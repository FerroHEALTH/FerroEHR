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
2. **Role mining** (`[auth]`): `FERROEHR__AUTH__ROLE_CLAIMS`
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
   export FERROEHR__AUTH__ISSUER=https://login.microsoftonline.com/<tenant-id>/v2.0
   export FERROEHR__AUTH__AUDIENCE=api://ferroehr
   # Entra ID mints no RFC 9068 tokens (`typ` JWT, the scopes in `scp`).
   export FERROEHR__AUTH__TOKEN_PROFILE=jwt
   export FERROEHR__AUTH__ROLE_CLAIMS='["roles"]'
   ```

5. **Verify**: request a token for the app (any OAuth2 client credential or
   auth-code flow) and call the API. Read the status codes as a diagnostic:

   | Status | What it tells you |
   |---|---|
   | `200`-family | roles arrived and the operation was permitted |
   | `401` | no credential, or one the server rejected (the body deliberately never says which) |
   | `403` | the token is valid but carries no role or SMART on openEHR scope the operation needs: a clinical call needs a matching scope and, for a token with no SMART on openEHR scope, at least one role, an admin call needs the admin role and the product scope |
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
   export FERROEHR__AUTH__ISSUER=https://adfs.example.com/adfs
   export FERROEHR__AUTH__AUDIENCE=ferroehr-api
   # AD FS mints no RFC 9068 tokens.
   export FERROEHR__AUTH__TOKEN_PROFILE=jwt
   export FERROEHR__AUTH__ROLE_CLAIMS='["role"]'
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
