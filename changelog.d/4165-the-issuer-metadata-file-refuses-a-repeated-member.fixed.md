- `[smart] issuer_metadata_file` is read by the same parse as metadata
  fetched from an issuer (#4165), so a file that repeats a member name is a
  boot error. It used to be read as it stood, keeping whichever value the
  parser took.
