- The `FERROEHR_ACCESS_CONTROL_V1` scheme of `EHR_ACCESS.settings` (its access
  list, privacy levels and single gate-keeper) is removed with no conversion:
  a body of it is refused with `400`, and an upgrade drops every stored
  `EHR_ACCESS` that carries it, with its history. The next boot gives each
  such EHR the default `EHR_ACCESS` without settings (#5508).
