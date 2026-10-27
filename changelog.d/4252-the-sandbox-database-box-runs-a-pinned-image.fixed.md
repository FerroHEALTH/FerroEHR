- **The hosted sandbox's database box runs a pinned image** (#4252).
  `deploy/hosted/cloud-init-postgres.yaml` defaults `FERROEHR_POSTGRES_IMAGE`
  to `ghcr.io/ferrohealth/ferroehr-postgres:5.4.2` by its index digest, where
  the box ran the moving `latest` tag of the image's pre-FerroHEALTH
  namespace, and `compose-image-tags.sh` holds the default and the hosted
  README to one release reference. Step 6 of `sandbox-ferropix-rollout.sh`
  moves the box onto the pin: it refuses a change of PostgreSQL major,
  recreates the container on its data volume and checks the CDR's readiness
  and a `ferropix` login afterwards. Step 1 of the script no longer prints an
  empty line for each container that is absent.
