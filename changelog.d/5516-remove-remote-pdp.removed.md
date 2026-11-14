- The remote policy decision point is gone: FerroEHR makes no outbound call to
  decide an access request. `[authz.abac.remote]` with its whole `endpoint` tree (`url`, the
  timeouts, `retry`, `tls` and `credentials` with `oauth` and `onward`), the
  `[authz.abac.policy.<name>]` bindings with `name` and `parameters`, and the
  `remote` value of `[authz.abac] engine` are unknown keys that refuse the
  start, from the file and from the environment. The
  `authz_remote_pdp_calls` metric and the egress rule the book listed for the
  policy service are gone too. The configuration reference lists 741 keys,
  down from 855. Remove the tables from your configuration; nothing replaces
  them.
