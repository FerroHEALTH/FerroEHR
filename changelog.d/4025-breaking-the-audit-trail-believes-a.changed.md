- **Breaking: the audit trail believes a forwarded client address only from a
  trusted proxy** (#4025). The source network address of an audit record
  (`client_ip`, the DICOM `NetworkAccessPointID`, the FHIR `AuditEvent`
  network) is the address the family server stack names: the connection's
  peer, or the client a peer listed in `[server] trusted_proxies` names in
  the `[server] forwarded_header` (RFC 7239 §8.1). FerroEHR read the first
  `X-Forwarded-For` hop from any caller, so a client could write any address
  into the trail. A deployment behind a reverse proxy lists the proxy in
  `trusted_proxies` and sets `forwarded_header = "x-forwarded-for"` when the
  proxy does not send `Forwarded`; until then its records name the proxy.
