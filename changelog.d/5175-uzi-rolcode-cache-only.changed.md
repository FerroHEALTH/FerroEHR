- The source tree no longer carries the UZI role code CodeSystem
  (`uzi-rolcode`) of Nictiz's zib 2020 package: its codes and titles
  reproduce the UZI-register CPS, which CIBG does not license for
  reproduction. `scripts/vendor/nl-role-codes.sh` writes it into the
  git-ignored `.vendor-cache/nl-role-codes/`, as it does the CPS, and
  `--test-inputs` fetches only what the Wabvpz tests read. Nictiz's care
  provider types stay under `docs/specs/nl-role-codes/`. Run
  `scripts/vendor/nl-role-codes.sh --test-inputs` once before running the
  `ferroehr-rest` tests.
