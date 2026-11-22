- A care relationship or `PERSON` commit that changes no login-binding row
  no longer moves the binding revision or notifies `ferroehr_party_binding`,
  so replicas stop dropping and re-reading their binding lookups after each
  one. Only a statement that changes a key row, a membership, a
  representation or a key space's state moves the revision and notifies, once
  per transaction.
