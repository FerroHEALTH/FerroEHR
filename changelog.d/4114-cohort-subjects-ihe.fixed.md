- **Cohort queries work under the IHE patient identity source.** That
  provider keeps no party-to-EHR map, so a cohort selected by a demographic
  predicate found no EHR. `POST /query/cohort` now takes `subjects`,
  identifiers in any domain, each resolved through the `[ehr_index]`
  provider: under the IHE provider by a PIXm (ITI-83) translation forward into
  the server's clinical domain, never back. The new `[cohort] subjects = true`
  turns the surface on without a predicate. Under the IHE provider a
  predicate is refused `400`, and a failing identity source fails the query
  instead of shrinking the cohort. The cohort digest takes each subject by
  namespace only.
