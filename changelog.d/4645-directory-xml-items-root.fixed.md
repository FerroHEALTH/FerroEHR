- A canonical XML FOLDER, EHR_STATUS, PERSON, AGENT, GROUP, ORGANISATION,
  ROLE or PARTY_RELATIONSHIP document now has the root element `items` with
  `xsi:type` naming its class, the element the ITS-XML schemas publish for a
  LOCATABLE (`Structure.xsd`). The directory reads (latest, by version and at a
  time), the EHR_STATUS reads, the demographic reads and the demographic
  template examples served `<folder>`, `<ehr_status>`, `<person>` and the like,
  which no ITS-XML schema declares. XML input accepts the same form.
