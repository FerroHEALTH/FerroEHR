- With the role gate on (`[authz.rbac] enabled`), a bearer token that carries a
  SMART on openEHR resource scope, such as `system/template-*.r`, now reaches a
  clinical operation without a role: its scopes decide what it reaches. A
  token with no SMART on openEHR scope still needs at least one role, and an
  admin operation still needs the admin role.
