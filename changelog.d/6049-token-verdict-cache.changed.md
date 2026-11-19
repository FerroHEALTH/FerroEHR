- A bearer token's signature is verified once and its verdict kept until the
  token's `exp`, through `ferrohealth-smart` 0.30.1: the same token on a
  later request is not verified again while its key is still in the
  issuer's key set, and a token past its `exp` or signed by a key the issuer
  no longer publishes is refused as before. The binding, `iat` and the
  classification are still checked on every request, and an introspected
  issuer is unchanged. The verdicts held are reported under
  `cache="token_verdicts"` of `cache_entries` and `cache_size_bytes`, at most
  10 000 tokens in 16 MiB per issuer.
