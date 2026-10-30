- The Simplified Flat `DV_INTERVAL` suffixes `|lower_unbounded`,
  `|upper_unbounded`, `|lower_included` and `|upper_included` are now read on
  an interval whose bounds are template nodes, and the flat output writes an
  unbounded flag when true and an included flag when false. The suffixes were
  refused as unknown with `422`.
