- **FerroEHR holds no identity logic** (#3380): `link_as_subject`, the
  server-side subject pseudonym (`mint_subject_pseudonym`), the party merge
  and split, and `resolve_ehr_for_identity`, the national-identifier-to-EHR
  crossing, are gone. Matching, cross-referencing identifier domains and
  minting pseudonyms belong to the identity source.
