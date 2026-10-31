- CVE-2026-84243, the glibc gettext `LANGUAGE` directory traversal
  (GLIBC-SA-2026-0025), has no Debian fix for trixie yet and is judged
  `not_affected` in every image. The `ferroehr` and `ferroehr-viewer`
  programs never leave the C locale, so glibc ignores `LANGUAGE` and opens no
  message catalog (`security/vex/ferroehr-os.openvex.json`). In
  `ferroehr-postgres` only the container environment the operator writes can
  set `LANGUAGE`; no database connection can
  (`security/vex/postgres-os.openvex.json`).
