- **The bundled PostgreSQL image is rebuilt on the 2026-10-21 respin of
  `postgres:18.6`** (#4084, `sha256:edb0c08d…`), a same-version respin with
  rebuilt base packages. The respin ships the fixed xz packages, so the
  `TEMP-1147318-639065` statement leaves `security/vex/postgres-os.openvex.json`.
