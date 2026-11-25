- An AQL query judged on an access copy older than the store is refused
  with `503 access-refreshing` only while the store keeps moving past each
  fresh read. A slow read of the copies on a store that holds still now
  answers the query.
