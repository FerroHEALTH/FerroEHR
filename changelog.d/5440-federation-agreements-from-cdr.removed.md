- **Breaking: `[authz.federation]` and its `[[authz.federation.agreement]]` tables are
  removed from `ferroehr.toml`** (`issuer`, `accessor`, `name`, `version`, `jurisdiction`,
  `exchange`, `purpose`, `legal_ground`, `consent`, `reads`, `writes`, `discard`,
  `discard_window`, `ehdsi_service`, `caller_rate`, `opt_out_override`, `access_class`,
  `chained_log_retention`). A configuration that still holds them refuses boot as an
  unknown key; write each agreement to the CDR as the book's "Operational policy" page
  describes. The Helm chart 28.0.0 drops `config.authz.federation.agreement` (a major
  bump: a values file that sets it is refused).
