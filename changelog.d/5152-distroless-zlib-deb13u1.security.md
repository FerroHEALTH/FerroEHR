- The `ferroehr` and `ferroehr-viewer` images move to the
  `gcr.io/distroless/cc-debian13:nonroot` build of 2026-11-08
  (`sha256:cc66d222…`), whose zlib1g `1:1.3.dfsg+really1.3.1-1+deb13u1`
  fixes CVE-2026-76844 and CVE-2026-85091 (HIGH, heap overflows in zlib's
  gzip writer) and CVE-2026-27171 (MEDIUM, an infinite loop in
  `crc32_combine64`). Neither program loads zlib. The images of 5.6.3 and
  earlier releases carry `1:1.3.dfsg+really1.3.1-1+b1`, judged `affected`
  with the upgrade as its action, as the `ferroehr-postgres` image of those
  releases is (`security/vex/ferroehr-os.openvex.json`) (#5152, #5329).
