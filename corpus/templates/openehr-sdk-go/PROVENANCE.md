# Provenance: openehr-sdk-go test corpus templates (excerpt)

- Source: <https://github.com/Cadasto/openehr-sdk-go>, commit `8362b0b9cd07ab214a31d3763fed922e7648363a`
- Upstream licence: the repository is MIT (`LICENSE`, Copyright (c) 2026
  Sebastian Iancu). Its corpus holds third-party material under each
  upstream's terms, recorded in `testkit/corpus/THIRD_PARTY_LICENSES.md`:
  the templates that come from EHRbase and the openEHR_SDK are Apache-2.0
  (`licenses/Apache-2.0.txt`), the CODE24 templates are MIT. Both files are
  vendored beside the templates.
- `blood_pressure_strict_existence.opt` is CODE24's (MIT): the blood
  pressure OBSERVATION of an Ocean Template Designer `vital_signs` export,
  with each existence as exported. `Referral Request.v1.opt` is CODE24's
  (MIT) as well.
- `cluster-slot.ehrbase.org.v0.opt` (from the openEHR_SDK validation tests)
  and `clinical_content_validation.opt` (from the EHRbase integration tests,
  with the one schema edit THIRD_PARTY_LICENSES.md records) are Apache-2.0,
  Copyright Vitasystems GmbH and Hannover Medical School.
- Layout: the upstream paths, unchanged
- Fetched by: `scripts/vendor/openehr-sdk-go-templates.sh`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)
- Consumer: `crates/openehr-sdt/tests/it/example_committable.rs` generates the
  `required` and `medium` examples of every template here and validates them

| file | sha256 |
|---|---|
| `LICENSE` | `e6ccba1c7cd1325461ca1e7d04e896a94100533fbb7a3abb7a3b8b37ee836864` |
| `licenses/Apache-2.0.txt` | `016bb40020346551fc8ffa26ba37bc20d33fd102bceef62c54757624ed1d76e5` |
| `testkit/corpus/THIRD_PARTY_LICENSES.md` | `9d751178c0d34d58b40338261418a19ef3902d33785d995b03bba6344968b881` |
| `testkit/corpus/templates/blood_pressure_strict_existence.opt` | `a346b96ee34aa423dc37c4459e87d14f59003715dc57efaff36b8b475296f0e7` |
| `testkit/corpus/templates/cluster-slot.ehrbase.org.v0.opt` | `40024665e6c8d9c50b8b5e59ddc261d2a030f07d86009458d3e7ad3948546b69` |
| `testkit/corpus/templates/clinical_content_validation.opt` | `fc22488a22ca5ebe39a6727b2fb4e6879f547e882d42f9a2006a73f57f789b08` |
| `testkit/corpus/templates/Referral Request.v1.opt` | `bd82e06d003b4708a83bde8df19f80b81c2e5da8b3595690c097822feeebadbf` |
