- The five OpenSSL 3.5.7 CVEs of the OpenSSL advisory of 2026-11-10 have no
  Debian fix for trixie and are judged `not_affected` in the three images:
  the AES-GCM-SIV flaws CVE-2026-88823 and CVE-2026-95692, the QUIC flaws
  CVE-2026-88822 and CVE-2026-95691, and the `CMS_encrypt()` loop
  CVE-2026-107496. The `ferroehr` and `ferroehr-viewer` programs never map
  `libssl.so.3` or `libcrypto.so.3`: each links only `libc`, `libm` and
  `libgcc_s`, imports no `dlopen`, and speaks TLS through rustls
  (`security/vex/ferroehr-os.openvex.json`). The PostgreSQL server maps
  both for TLS over TCP, which never selects AES-GCM-SIV (no TLS cipher suite
  is one), never opens a QUIC connection and never writes CMS, and the image
  never runs the `openssl` program or loads the legacy provider
  (`security/vex/postgres-os.openvex.json`).
