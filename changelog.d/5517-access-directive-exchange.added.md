- The controller imports and exports an EHR's access directives as HL7 FHIR
  `Consent` (R4 and R5), IHE BPPC acknowledgements and IHE APPC privacy
  consent documents under `{ext}/admin/ehr-access/fhir/{release}/Consent`,
  `{ext}/admin/ehr-access/bppc` and `{ext}/admin/ehr-access/appc`. An import
  commits a new `EHR_ACCESS` version over the one `If-Match` names, appended
  to the directives in force; a document that holds what no directive can
  state, such as a provision nested deeper than one exception level,
  `provision.data.meaning` other than `instance`, R5 `provision.expression`,
  a BPPC or APPC policy the `ACCESS_POLICY` catalogue does not hold, or an
  XACML combining algorithm other than deny-overrides, is refused whole with
  the element named. A FHIR action coded outside FerroEHR's terminology is
  translated through the configured terminology server. Export writes every
  directive attribute with no FHIR core element in a FerroEHR extension. No
  configuration key is added or removed.
