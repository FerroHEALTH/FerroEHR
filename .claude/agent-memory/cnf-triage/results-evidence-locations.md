---
name: results-evidence-locations
description: Where the CNF observation, catalogue, and spec text live for triage
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

For the committed ferroehr run (a Veredictum 0.3.3 released-profile record),
per-row observation is in `docs/conformance/ferroehr/results.json` under
`rows[]`: each entry has `case`, `row`, `status` (passed, failed, warning,
errored, skipped), its `inputs`, and for a skipped row `skip.reason` with the
citation. `outline.cases` places each case (chapter, folder, BASE or option
standing, TRIAL mark). `summary.md` beside it opens every red row up to the
step, expected against received, and the cited sentence. The exchanges
themselves are in `recording.har`, which is never committed.

- Case cores: Veredictum's `artifacts/schedule/<family>/<CASE>.yaml`
  (SM operation + outcome kinds + spec_refs + often a self-documenting TODO
  naming the real defect).
- Operation bindings (wire realization): Veredictum's `artifacts/bindings/its-rest/<SM.op>.yaml`.
- Corpus: `corpus/MANIFEST.yaml` maps data-set
  aliases (`cnf.*`) → fixture files + `template_id`; fixtures under
  `corpus/fixtures/**`, templates under `corpus/templates/**`.
- Party/statement: `docs/conformance/party/ferroehr/{statement.json,ixit.json}`
  — `statement.options` holds the selected `option_select` branches.
- Spec oracle: `docs/specs/openehr/` (index in its README). SF cases →
  `ITS-REST/docs/simplified_formats/master0{2,4,5,6}-*.adoc`. ADL14 →
  `CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc` +
  `ITS-REST/specifications/parameters/path/template_id.yaml`.
