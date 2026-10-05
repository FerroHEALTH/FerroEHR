# Service fixture corpus

Beside this record the directory holds 108 fixture files: 92 OPT 1.4
operational templates, 7 ADL 1.4 archetypes, 5 JSON bodies, 3 XML samples and
one AQL query. They are the upload-surface and round-trip material for the
service, REST and ITS suites, and the `.json`, `.xml` and `.aql` files are also
symlinked into the fuzz seed corpus by `fuzz/seeds.sh`.

## Where the material comes from

106 of the 108 files are EHRbase's own `service/src/test/resources/`, taken
verbatim with the fork import at **EHRbase v2.33.0** — the commit
`Import: EHRbase as the fork point`,
`5d93ba82f735f768034986f488a000d78fb24714`, recorded in `docs/VERSIONS.md`
§EHRbase reference point. Upstream repository:
<https://github.com/ehrbase/ehrbase>. Measured on 2026-09-16 by comparing
every file in this directory against its blob at that commit: none of the 106
has been edited since the import.

Two files were written in this repository:

| file | origin |
|---|---|
| `knowledge/opt/minimal_evaluation.opt` | a minimal EVALUATION template built here beside EHRbase's `minimal_observation.opt`, for the `C_DV_QUANTITY` conversion path |
| `knowledge/archetypes/openEHR-EHR-OBSERVATION.revision_history.v1.adl` | an ADL 1.4 regression archetype written for the dADL reading surface, its `original_author` naming this project's regression corpus |

## Licence

Apache-2.0, the licence of the EHRbase repository these files were imported
from, which is the only licence stated anywhere for them: measured on
2026-09-16, 91 of the 92 OPT files carry an `other_details id="Copyright"`
element and every one of those 91 is EMPTY (the 92nd carries no such element
at all), the single `other_details id="licence"` element in the set (in
`non_unique_aql_paths.opt`) is empty as well, and none of the 7 archetypes
carries a `licence` or `copyright` field in its `description` block. The templates name clinical-model authors in their
metadata (Ocean Informatics, Ripple, IDCR, COLNEC and others) without stating
terms, so the tree licence is the upstream repository's and the copyright
holders named in `REUSE.toml` are the openEHR Foundation, the EHRbase
contributors and Cadasto B.V. for the two files written here.

## Maintenance

Hand-maintained, not script-fetched. There is no vendor script for this
subtree and there will not be one: it is a single historical import plus
locally authored additions, so there is nothing to re-fetch. A file added
here states its origin in the table above; a file is never hand-edited, in
line with `.claude/rules/vendored-corpora.md`.

Five files were deleted when the tree moved here from
`app/ferroehr/tests/resources/service` (issue #3408): an EHRbase persistence
expectation fixture, a Spring Boot server configuration, two zero-byte
directory placeholders and an `.oet` template source no reader in this
repository can parse. Nothing named or swept by a test was touched.
