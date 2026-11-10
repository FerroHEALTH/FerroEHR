- Thirteen MIT Kerberos 5 CVEs of MITKRB5-SA-2026-001 in the `ferroehr-postgres`
  image's `libgssapi-krb5-2`, `libk5crypto3`, `libkrb5-3` and
  `libkrb5support0` `1.21.3-5+deb13u1` have no Debian fix for trixie and are
  judged `not_affected`. The image keeps the four libraries because the
  `postgres` binary links `libgssapi_krb5.so.2`. Eleven sit in code the image
  does not carry: the KDC (CVE-2026-107364), its PKINIT plugin
  (CVE-2026-71200), `kadmind` (CVE-2026-107884, CVE-2026-112318),
  `libgssrpc` (CVE-2026-112314, CVE-2026-112315, CVE-2026-112320,
  CVE-2026-118380), the kadm5 libraries (CVE-2026-112316, CVE-2026-112319)
  and `libkdb5` (CVE-2026-112317). The PAC checksum flaw (CVE-2026-111423)
  and the CAMMAC checksum flaw (CVE-2026-71199) sit in `libkrb5`, which the
  server reaches only when it accepts a Kerberos ticket. The configuration
  FerroEHR ships has no `gss` or `hostgssenc` line in `pg_hba.conf` and no
  server keytab, and the server never reads a PAC or an authentication
  indicator (`security/vex/postgres-os.openvex.json`).
