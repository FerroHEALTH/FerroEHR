- With the role gate on (`[authz.rbac] enabled`), a bearer token whose SMART
  on openEHR resource scopes are all `system/` scopes, such as
  `system/template-*.r`, now reaches a clinical operation without a role: it is
  a backend client, and its scopes decide what it reaches. A `user/` or
  `patient/` token belongs to a person and still needs at least one role, as
  does a token with no SMART on openEHR scope and a Basic user; an admin
  operation still needs the admin role.
