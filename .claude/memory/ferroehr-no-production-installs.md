---
name: ferroehr-no-production-installs
description: As of 2026-11-01 no installation runs FerroEHR in production, so upgrade work for old data (5.5.0 audit records, data migrations) is not worth doing
metadata:
  type: project
---

As of 2026-11-01 no installation runs FerroEHR in production. 5.5.0 and the 5.6.0 release candidates run only on test and partner sandbox deployments, which can be recreated.

**Why:** the owner, 2026-11-01, after the orchestrator filed #4701 to convert 5.5.0 audit records in place: "there is no body running this currently in a real production way". #4701 was closed as not planned. The decision of #4300 stands: a 5.5.0 audit store that holds records is refused by audit migration 0017, and a test install is recreated.

**How to apply:** do not design or build data-preserving upgrade paths for old FerroEHR data until the owner says a production install exists. Released migration files still never change, because the immutability guard and sqlx checksums bind every installed database, test ones included. Before filing upgrade-path work, ask whether anyone runs the old version in production. Related: [[no-compat-shims]].
