---
name: results-evidence-locations
description: Where the CNF observation, catalogue, and spec text live for triage
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

For the committed ferroehr run (a Veredictum 0.4.1 released-profile record, judged on the stable profile),
per-row observation is in `docs/conformance/ferroehr/results.json` under
`rows[]`: each entry has `case`, `row`, `status` (passed, failed, warning,
errored, skipped), its `inputs`, and for a skipped row `skip.reason` with the
citation. `outline.cases` places each case (chapter, folder, BASE or option
standing, TRIAL mark). `summary.md` beside it opens every red row up to the
step, expected against received, and the cited sentence. The exchanges
themselves are in `recording.har`, which is never committed.

- Cases: the Arazzo workflows under Veredictum's
  `catalogue/cases/<folder>/<document>.arazzo.yaml` at the pinned tag, each
  with its `x-veredictum-cite` (source, section, quote, `report:`); setups
  under `catalogue/cases/_setup/`; option arms in `catalogue/capabilities.yaml`.
- Corpus: `corpus/PROVENANCE.md` indexes the shared tree: fixtures under
  `corpus/fixtures/**`, templates under `corpus/templates/**`, the
  catalogue's ADL 2 content cases under `corpus/veredictum/adl2-content/`.
- Party/statement: `docs/conformance/party/ferroehr/{statement.json,ixit.json}`
  — `statement.options` holds the selected option arms.
- Spec oracle: `docs/specs/openehr/` (index in its README). SF cases →
  `ITS-REST/docs/simplified_formats/master0{2,4,5,6}-*.adoc`. ADL14 →
  `CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc` +
  `ITS-REST/specifications/parameters/path/template_id.yaml`.
