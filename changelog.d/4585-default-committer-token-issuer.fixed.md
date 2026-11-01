- A write that sends no committer, on a route that takes a bare RM body (the
  `EHR_STATUS`, COMPOSITION and DIRECTORY writes, and the demographic party
  and relationship writes), now records a Bearer caller's login as issued by
  the token issuer, as EHR creation and every other default committer already
  did. It recorded the issuer `ferroehr` for every caller before; a Basic
  caller's login is still issued by `ferroehr`.
