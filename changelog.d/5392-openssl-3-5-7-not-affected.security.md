- Three OpenSSL 3.5.7 CVEs of the advisory of 2026-11-10 in the `ferroehr`
  and `ferroehr-viewer` images' `libssl3t64` `3.5.7-1~deb13u3` have no
  Debian fix for trixie and are judged `not_affected`: the AES-GCM-SIV flaws
  CVE-2026-88823 and CVE-2026-95692 in `libcrypto` and the QUIC flaw
  CVE-2026-95691 in `libssl`. Neither program maps `libssl.so.3` or
  `libcrypto.so.3`: each links only `libc`, `libm` and `libgcc_s`, imports
  no `dlopen`, and speaks TLS through rustls
  (`security/vex/ferroehr-os.openvex.json`).
