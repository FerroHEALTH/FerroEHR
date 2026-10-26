- `ferrohealth-smart` 0.7.1 masks the userinfo of every URL in its grant
  tables, a URL no parser accepts included. With 0.7.0 a configured
  `token_endpoint` or `issuer` that did not parse could reach the
  configuration renderings and the log in full (CWE-532,
  GHSA-vphj-3f8c-5hpx).
