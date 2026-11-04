- The terminology overlay (`docker-compose.terminology.yml`), the Helm chart
  and the hosted sandbox run FerroTERM 0.3.3
  (`ghcr.io/ferrohealth/ferroterm:0.3.3@sha256:f25ddad886fd218ae1eb82af6b81b8ed16651333f5bd802e621c5ae83b0913f1`),
  where the overlay and the chart ran 0.1.6 and the sandbox 0.3.1. FerroTERM
  0.3 reads only its `FERROTERM__…` configuration tree, so the overlay and the
  chart now set `FERROTERM__TERMINOLOGY__CODE_SYSTEMS`,
  `FERROTERM__TERMINOLOGY__INDEX`, `FERROTERM__TERMINOLOGY__DEFAULT_LANGUAGE`
  and `FERROTERM__TELEMETRY__FORMAT`. The overlay's own `FERROTERM_INDEX`,
  `FERROTERM_LOG_FORMAT` and `FERROTERM_DEFAULT_LANGUAGE` shell variables keep
  their meaning. The index layout is unchanged, so a built index serves as it
  is. Upgrade: move any `terminology.extraEnv` entry that sets a pre-0.3
  FerroTERM variable to its `FERROTERM__…` key.
