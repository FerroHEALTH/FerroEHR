- A population AQL query that only counts its root objects, such as
  `SELECT COUNT(c) FROM EHR e CONTAINS COMPOSITION c` or the same with
  `COUNT(*)`, reads the current version rows and no longer probes the root
  node of every composition. At 545 000 compositions the count fell from about
  9 s to 0.2 s.
