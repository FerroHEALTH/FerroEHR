- `OPTIONS` on the API base path sends `conformance_profile` as
  `BASE+OPTIONS` where it sent `STANDARD`. The build derives the value from
  the committed Veredictum record, `docs/conformance/ferroehr/summary.md`:
  `BASE+OPTIONS` when the BASE rows and the run OPTIONS rows have none failed
  and none errored, `BASE` when only the BASE rows do, and the member left
  out otherwise. ITS-REST 1.1.0 types the member as a free string and CNF
  names no profiles, so the values are FerroEHR's own. The
  `[rest.identity] conformance_profile` configuration key is removed: a
  configuration that still sets it is refused at boot as an unknown key.
