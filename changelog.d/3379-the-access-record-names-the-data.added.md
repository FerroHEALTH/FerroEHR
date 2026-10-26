- **The access record names the data permit a secondary-use caller acts
  under** (#3379). A caller declares the permit's identifier in the header
  `[audit] permit_header` names (default `x-data-permit`) beside its purpose
  of use; every access record keeps it as `permit`, so the trail says under
  which permit a population query, an export or a cohort query ran (EHDS
  Art. 68). A value that is not 1 to 128 characters of letters, digits and
  `.` `_` `:` `/` `#` `-` is recorded as absent. The column joins the tamper
  chain as digest version 4 (audit migration 0016). The FHIR and DICOM
  renderings do not carry the permit yet.
- **AQL takes `VERSIONED_OBJECT` as a FROM class** (#3379), the shape of
  FerroBRIDGE's OMOP batch query (`EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS
  VERSION v CONTAINS COMPOSITION c`). The QUERY specification lets `FROM` name
  any RM class (master03 §FROM), and FerroEHR refused this one. The variable
  answers `uid/value` and `owner_id/id/value` (RM common master06
  §VERSIONED_OBJECT) and refuses any other path. The operand sets no version
  scope of its own.
- **The book records the secondary-use contract with FerroBRIDGE** (#3379).
  The new Secondary use page states the batch query FerroBRIDGE loads the
  OMOP CDM from (`time_committed > $since`, ordered by the commit instant and
  the version uid, paged by `offset` and `fetch`), the marks that take an EHR
  out of a batch, the reconciliation that removes erased compositions, and
  where pseudonymisation for research happens. An integration test holds the
  batch to it. The compliance overview no longer names the withdrawn
  secondary-use read model (#3160) as planned.
