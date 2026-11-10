- CVE-2026-112321, CVE-2026-112322 and CVE-2026-112323 (memory and CPU
  exhaustion in `kadmind` through the AUTH_GSSAPI handler) and
  CVE-2026-119079 (CPU exhaustion in the KDC decoding an AS-REQ or TGS-REQ)
  in MIT Kerberos 5 have no Debian fix for trixie and are judged
  `not_affected` in `ferroehr-postgres`. The image carries neither `kadmind`,
  a KDC nor `libgssrpc`, and the PostgreSQL server never decodes either
  request; the configuration FerroEHR ships enables no GSS-API
  authentication (`security/vex/postgres-os.openvex.json`) (#5344).
