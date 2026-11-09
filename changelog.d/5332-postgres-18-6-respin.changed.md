- **The FerroEHR PostgreSQL image is built on the 2026-11-09 respin of
  `postgres:18.6`** (`sha256:e3d24f69…`), a same-version respin with rebuilt
  base packages. A full scan of the rebuilt image on amd64 and arm64 finds the
  same findings as the image of the previous base, each judged in
  `security/vex/postgres-os.openvex.json`, and a gate scan finds no fixable
  HIGH or CRITICAL finding.
