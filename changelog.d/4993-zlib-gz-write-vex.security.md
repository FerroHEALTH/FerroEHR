- CVE-2026-76844, a heap overflow in zlib's gzip file writer after a failed
  write, has no Debian fix for trixie and is judged `not_affected` in every
  image. The `ferroehr` and `ferroehr-viewer` programs never load the Debian
  zlib (`security/vex/ferroehr-os.openvex.json`), and in `ferroehr-postgres`
  the zlib 1.3.1 overflow needs a `gzclearerr()` call that nothing in the
  image makes (`security/vex/postgres-os.openvex.json`).
