- The `ferroehr-postgres` image carries zlib1g
  `1:1.3.dfsg+really1.3.1-1+deb13u1` from trixie-security, which fixes
  CVE-2026-76844 and CVE-2026-85091 (HIGH, heap overflows in zlib's gzip
  writer) and CVE-2026-27171 (MEDIUM, an infinite loop in
  `crc32_combine64`). The image of 5.6.3 and earlier releases carries
  `1:1.3.dfsg+really1.3.1-1+b1`, judged `affected` with the upgrade as its
  action (`security/vex/postgres-os.openvex.json`). The `ferroehr` and
  `ferroehr-viewer` images keep the distroless base's zlib, which neither
  program loads (`security/vex/ferroehr-os.openvex.json`) (#5137).
