---
name: extract-export-latest-only-default
description: A default EHR-Extract export is LATEST-ONLY by spec, so it cannot evidence a version stack — use POST /message/export with include_all_versions
metadata:
  type: project
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

`export_ehrs` (SM: takes no `EXTRACT_SPEC`; our route `GET
/message/export/{ehr_id}`) synthesizes a latest-only spec, and that is
spec-required, not a shortcut:

- `RM/docs/ehr_extract/master04-common_package.adoc` §Version Specification —
  "An Extract request in its simplest form has no version specification,
  corresponding to the assumption of **'latest available version'** for each
  matched item."
- `RM/docs/UML/classes/org.openehr.rm.ehr_extract.extract_version_spec.adoc`
  §Description — "**By default, only latest versions are included** in the
  Extract, in which case this part of the Extract specification is not needed
  at all."

So any case asserting that N version positions are HELD must not read them out
of a default export. Two instruments exist:

- `X_VERSIONED_OBJECT.total_version_count` in the SAME default export already
  states the held count ("Total number of versions in original VERSIONED_OBJECT"
  vs `extract_version_count` = "the count of items in the versions attribute").
- `export_ehr_extracts` (`POST /message/export`, binding
  `I_EHR_EXTRACT_SERVICE.export_ehr_extracts.yaml`) takes a whole EXTRACT_SPEC,
  so a case can set `include_all_versions: true`.

Reproduced 2026-08-03: the same imported branch tree serves
`extract_version_count 1` (GET default, trunk head only) and `3` (POST with
include_all_versions=true, trunk ::1 + branch ::1.1.1 + trunk ::2), with
`total_version_count 3` in both.

**How to apply:** a red `import_ehr_extract-*` row whose `returns:` regex misses
an older version uid or an older `time_committed` is a CATALOGUE defect (wrong
evidence instrument), not an import defect — check `total_version_count` in the
observed body first, it usually proves the import was correct.

Open register candidate: with a trunk head and a branch head coexisting, the
released text never says which is "the latest available version" for a
latest-only export; our SUT picks the trunk head, nothing pins it.
