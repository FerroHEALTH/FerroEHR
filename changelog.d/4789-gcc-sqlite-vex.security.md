- CVE-2026-114690, CVE-2026-114691 and CVE-2026-115455, three libstdc++
  defects in the gcc-14 packages of every image, have no Debian fix for
  trixie and are judged `not_affected`. Each fix changes only a header
  template (the parallel-mode search, the `std::vector<bool>` constructors,
  wide-stream extraction into `std::bitset`), so `libstdc++.so.6` holds no
  copy; the `ferroehr` and `ferroehr-viewer` programs contain no C++ code
  (`security/vex/ferroehr-os.openvex.json`), and in `ferroehr-postgres`
  nothing is built in parallel mode and PostgreSQL does not link libstdc++
  (`security/vex/postgres-os.openvex.json`).
- CVE-2026-119371 to CVE-2026-119379 and CVE-2026-119591, ten SQLite
  defects without a Debian fix, are judged `not_affected` in
  `ferroehr-postgres`: only `lslogins` reaches SQLite there, through
  liblastlog2, and nothing in the image runs it
  (`security/vex/postgres-os.openvex.json`).
