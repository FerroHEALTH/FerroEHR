# `Demonstration.v1`: a third-party regression fixture

`Demonstration.v1.opt` is the file
`internal/fixtures/templates/Demonstration.v1.opt` of
[Cadasto/cdr-benchmark](https://github.com/Cadasto/cdr-benchmark) 0.17.0,
commit `5f452acab7d78be9c879c8f806dc31ce21349e0c`, copied byte for byte
(sha256 `8ce36e73e001d3b55c660cfcc9b22434c3f20162da64608d7e38c13217ee9bf7`).
The repository is MIT-licensed (Copyright 2026 Cadasto B.V.). The template
inlines the openEHR CKM archetypes `openEHR-EHR-COMPOSITION.encounter.v1` and
`openEHR-EHR-OBSERVATION.demo.v1`, licensed CC-BY-SA 3.0 Unported by the
openEHR Foundation. `LICENSES/MIT.txt` and `LICENSE-CC-BY-SA-3.0` at the
repository root are the licence texts; the declaration is in `REUSE.toml`.

## Why it is here

The cdr-benchmark run against FerroEHR met two defects on this template:
an ELEMENT `null_flavour` constraint shown as a value alternative (#5769) and
a value on an ELEMENT with value alternatives dropped (#5768). The template
carries both shapes in one heading: "Boolean" (`at0016`) constrains `value`
to `DV_BOOLEAN` and `null_flavour` to the openEHR codes 271, 272, 273 and 253,
and "Choice" (`at0025`) allows `DV_QUANTITY` or `DV_CODED_TEXT`.
