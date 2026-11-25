- The PostgreSQL image and the compose files build on the rebuilt
  `postgres:19.0` (`sha256:aaa10f7b…`), a same-version respin with Debian's
  newer base packages; 35 statements of `security/vex/postgres-os.openvex.json`
  whose findings no longer fire on it are removed.
