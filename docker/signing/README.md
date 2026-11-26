# The development lanes' version-signing key

## THIS IS COMMITTED TEST KEY MATERIAL. IT IS NOT SECRET AND MUST NEVER BE USED IN PRODUCTION.

`version-signing-test.key.asc` is a passphrase-less ML-DSA-65+Ed25519 OpenPGP
secret key (RFC 9980 algorithm ID 30) in public version control. Anyone with
a checkout can sign as it. It exists for one purpose: the development lanes
run `[record.signing] mode = "pgp"`, so every version the demo dataset
commits carries a real ML-DSA-65+Ed25519 signature and the access log is
sealed into a signed checkpoint, which the viewer draws.

Never point a deployment that holds real data at this key, and never list its
public half among the `retired_key_files` of one. A signature made with it
proves nothing.

## How it was made

```bash
ferrohealth-sop generate-key --signing-only \
  "FerroEHR development lanes TEST KEY <signing-test@ferroehr.invalid>" \
  > docker/signing/version-signing-test.key.asc
```

`ferrohealth-sop` is `cargo install ferrohealth-openpgp-pq --version =0.1.0 --locked`.

## Who uses it

`docker/viewer/e2e-env.yml` mounts it into the CDR of the browser-journey
lane (`scripts/ui-e2e.sh`) as `/etc/ferroehr/version-signing-test.key.asc` and
sets `FERROEHR__RECORD__SIGNING__MODE=pgp`.
