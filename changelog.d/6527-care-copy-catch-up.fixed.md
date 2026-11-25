- A read or query that care admits, on a replica whose caller facts copy is
  catching up, no longer answers `500`. The copy is done catching up once a
  fresh read of the store finds it current, and a refusal while the store
  keeps moving answers `503 access-refreshing` with `Retry-After`.
