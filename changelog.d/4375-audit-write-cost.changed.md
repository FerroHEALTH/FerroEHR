- The access log costs the request path less under load. The audit store
  writes a drained batch, its search tokens and its token sets in one
  transaction on one connection (one commit where it took up to three), stores
  only the tokens it does not hold yet, and the GIN index on `search_tokens`
  takes new entries through a pending list of 512 kB. On the release load
  test the store wrote 1.5 KB of WAL per record where it wrote 5.2 KB.
