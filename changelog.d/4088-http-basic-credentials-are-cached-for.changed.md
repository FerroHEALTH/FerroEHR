- **HTTP Basic credentials are cached for 300 seconds by default** (#4088).
  `[auth.basic] cache_ttl_seconds` defaulted to `0`, so every Basic request
  ran the Argon2id derivation, 10 to 15 ms of CPU, which capped the mixed
  ITS-REST and AQL workload against a release build on one host at 408 op/s;
  with the cache it reached 1559 op/s. `ferrohealth-smart` 0.7.5 makes 300
  the default. Only a credential that verified is kept, under a keyed hash
  drawn at boot, and the cache empties at a restart. Set `0` to check every
  request. FerroEHR takes `ferrohealth-smart` 0.7.5, `ferrohealth-authz`
  0.1.5, `ihe-atna` and `ehds-logging` 0.2.13 and `ferrobridge-term` 0.1.139.
  Helm chart 16.0.1 states the new default.
