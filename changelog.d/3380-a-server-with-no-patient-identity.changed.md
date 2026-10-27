- **A server with no patient identity source refuses to start** (#3380).
  The binary's default `[ehr_index]` provider is the IHE one, and without
  `pixm_url` and `assigning_authority` the refusal names both choices:
  configure the identity source, or opt in to the in-instance map with
  `[ehr_index] provider = "local"`, which serves the
  `GET /ehr?subject_id=…` lookup over `EHR_STATUS` content and nothing more.
  Upgrade step: a deployment without an identity source adds
  `[ehr_index] provider = "local"` (`FERROEHR__EHR_INDEX__PROVIDER=local`).
- **The compose quickstart runs FerroPIX beside FerroEHR** (#3380), pinned by
  tag and digest, with its own PostgreSQL and a development OpenID Provider,
  and points `[ehr_index]` at it; FerroPIX mints the pseudonym in
  `urn:oid:2.999.7.1` on FerroEHR's first ask. The server now runs in the
  issuer's network namespace, which publishes ports 8080, 8090
  (`FERROEHR_PIX_PORT`) and 8180 (`FERROEHR_ISSUER_PORT`).
- **Helm chart 19.0.0 takes FerroPIX as the chart dependency `ferropix`**
  (#3380), on by default: the chart points `config.ehr_index` at its Service,
  adds the egress rule to its pods, and routes the grant's client secret
  through `secrets.ehrIndexClientSecret`. A values file that names no identity
  source is refused at render. Upgrade step: supply FerroPIX's
  `ferropix.database.existingSecret`, `ferropix.dataKey.existingSecret`, its
  issuer and pseudonymisation domain, and `config.ehr_index`, or set
  `ferropix.enabled: false` with `config.ehr_index.provider: local`.
