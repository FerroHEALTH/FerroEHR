- **The book and the architecture no longer claim the openEHR Service Model as
  the design of the service layer** (#4211). The service layer is FerroEHR's
  own design, judged by what goes in and comes out (BASE, RM, AM, TERM,
  QUERY/AQL, ITS-REST, ITS-JSON, ITS-XML and the serialisation formats); the
  vendored SM text stays as reference only. No behaviour changes.
