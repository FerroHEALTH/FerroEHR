- `[authz.professional]` (`claim`, `client_tokens`, `acting_for_claim`) and
  its `403` for a token that names no natural person. The issuer's `kind`
  and profile classify the caller, and the access record's acting mode and
  professional come from that classification: a person's own token is the
  person, and a delegated token is the client acting for the person it
  names. Upgrade step: remove the table, which refuses the start, and list a
  client that acts for a person in its issuer's `delegates`.
