- A `[server.tls]` table that sets `client_ca_file` with `client_auth` at
  `none`, written or by default, is refused at load, naming the setting
  (GHSA-52mf-qg39-4q4j). Before 5.5.0 a client CA alone meant that every
  client presents a certificate; 5.5.0 moved the listener onto
  `ferrohealth-server`, where `client_auth` defaults to `none` and the CA was
  read and ignored, so a configuration carried over passed `config check` and
  served without asking a client certificate. FerroEHR takes
  `ferrohealth-server` 0.3.8, which refuses it. A 5.5.0 deployer who sets
  `client_ca_file` sets `client_auth = "required"` beside it. The default
  template no longer pairs a client CA with `none`, and says what each
  `client_auth` value does.
