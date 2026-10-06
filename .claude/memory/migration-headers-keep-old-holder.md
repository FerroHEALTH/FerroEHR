---
name: migration-headers-keep-old-holder
description: "Owner 2026-10-06 — released migrations keep their \"Vernum Projecten B.V.\" SPDX header; never edit them or add a REUSE override, the sqlx checksum would fail"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 7514b6fd-a08b-4154-8327-9b3929e46f2e
  modified: 2026-10-06T12:27:36.723Z
---

The migration files under `app/ferroehr/migrations/` present at a release tag keep `SPDX-FileCopyrightText: Vernum Projecten B.V.` even though the holder is now Cadasto B.V. Owner, 2026-10-06: "they stay because otherwise the DB checksum fails". No REUSE.toml override either.

**Why:** sqlx records each applied migration's checksum; editing a released file breaks every existing database at boot.
**How to apply:** never flag these headers as a defect to fix; new migrations carry `Cadasto B.V.`. Related: [[migrations-are-append-only]], [[license-busl]].
