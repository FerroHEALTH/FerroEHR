---
name: postgresql-19
description: every FerroHEALTH product with a database runs, tests against and documents PostgreSQL 19; 18 and older are dropped (owner, 2026-11-21)
metadata:
  type: project
---

Every FerroHEALTH product with a database runs on, tests against and
documents PostgreSQL 19, the latest major (owner, 2026-11-21). PostgreSQL 18
and older are dropped with no upgrade path ([[ferrohealth-not-in-production]]).
The image is pinned by tag and digest in the repository's pin matrix, and a
new PostgreSQL release is taken as a pin change. A third-party system in a
test harness keeps the image its vendor documents.

**Why:** one database major across the family, always the latest.

**How to apply:** every compose file, test container, CI service and page
that names a PostgreSQL version names the pin row. A product that checks the
server version at connect refuses a server below 19.
