- An OpenPGP signing key protected by a passphrase is unlocked once, when the
  server loads it, and every version and access-log link is signed without
  deriving the key again. Each signature used to run the key's deliberately
  slow string-to-key derivation (RFC 9580 §3.7), which with signed access-log
  links cost about 75 ms of CPU per signature, inside the audit writer's
  transaction. The server no longer keeps the passphrase.
