- **The FerroEHR PostgreSQL image is built on the 2026-11-02 respin of
  `postgres:18.6`** (#4836, `sha256:3a426452…`), a same-version respin with
  rebuilt base packages. Every finding of a full scan of the rebuilt image
  keeps its judgement in `security/vex/postgres-os.openvex.json`, and none
  goes stale.
