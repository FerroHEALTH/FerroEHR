- `[auth]` takes three boot refusals from `ferrohealth-smart`: a Basic
  user's hash below the Argon2id floor (m=19456, t=2, p=1), an issuer that
  carries `userinfo`, and private or symmetric key material in a
  `jwks_file`. They are refused as before, with the crate's messages. A
  `jwks_file` that is no JWK Set now refuses the start where the crate
  reads the key set.
