- The chart's `terminology.ui` value and the terminology overlay's
  `FERROTERM_UI` variable: FerroTERM 0.3 serves no UI of its own, so neither
  had an effect. A values file that still sets `terminology.ui` is refused by
  the values schema; delete the key.
