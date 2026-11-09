- The EHR index's boot check reads the PIX Manager's `CapabilityStatement`
  through `ihe-iti`'s PIXm client, with its transport and its
  client-credentials token, so a Manager behind IUA answers the probe as it
  answers ITI-83. A failed probe is reported as the client's refusal: no
  answer, the status it answered, or no FHIR R4 `CapabilityStatement`.
