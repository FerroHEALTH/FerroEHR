- `OPTIONS` on the API base path no longer sends `conformance_profile`. The
  ITS-REST 1.1.0 `Options` schema makes the member optional, and no
  conformance record FerroEHR publishes assigns a profile. The
  `[rest.identity] conformance_profile` configuration key is gone with it: a
  configuration that still sets it is refused at boot as an unknown key.
