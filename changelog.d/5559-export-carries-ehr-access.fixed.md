- An EHR Extract exported under an `export_ehr_extract` grant covering the
  EHR, such as a migration export, carries the EHR's `EHR_ACCESS` again, so an
  import elsewhere keeps the person's access directives. An extract for any
  other caller who may not read the settings still leaves it out (#5559).
