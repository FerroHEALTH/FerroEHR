# Provenance: the IHE PIXm FHIR package (excerpt)

- Source: <https://packages.fhir.org/ihe.iti.pixm/3.1.0>, the FHIR package registry's copy of the IG published at
  <https://profiles.ihe.net/ITI/PIXm/3.1.0/>
- Pin: package `ihe.iti.pixm` version `3.1.0`, tarball sha256 `19e2e8eaf3030ac7b4d809c5e1eeb8face02c8635318aeb6d35bc2bb889de0d0`
- Upstream licence: Creative Commons Attribution 4.0 International
  (`CC-BY-4.0`, the `license` of the package manifest;
  <https://creativecommons.org/licenses/by/4.0/>). The package ships no
  licence file of its own. Attribution: IHE International, IT Infrastructure
  Technical Committee, *Patient Identifier Cross-referencing for Mobile
  (PIXm)* 3.1.0.
- Layout: the upstream paths inside the package, unchanged
- Files: package/OperationDefinition-IHE.PIXm.pix.json package/CapabilityStatement-IHE.PIXm.Manager.json package/example/Parameters-pixm-request-mohralice-red-to-blue.json package/example/Parameters-pixm-response-mohralice-red-to-blue.json package/example/Parameters-pixm-response-mohralice-red-all.json package/example/OperationOutcome-pixm-response-error-not-found.json 
- Fetched by: `scripts/vendor/ihe-pixm.sh`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)
- Consumer: `app/ferroehr/tests/it/ehr_index_ihe.rs` serves the example
  answers from a stub PIX Manager to the EHR index's identity source
