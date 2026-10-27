- The hosted sandbox (sandbox.ferroehr.eu) resolves patients through FerroPIX
  0.0.10, the FerroHEALTH patient identity source, as the compose quickstart
  does: FerroPIX runs beside the CDR on loopback with its database on the
  database box, each `EHR_STATUS` names its patient by the pseudonym FerroPIX
  minted in the clinical domain `urn:oid:2.999.7.1`, and `POST /query/cohort`
  takes `subjects` in the synthetic hospital domain `urn:oid:2.999.1.1`. The
  nightly reseed registers the demo patients with FerroPIX through the deploy
  script's new `identities` verb before it seeds, so after a reset the CDR's
  EHRs and FerroPIX's identities agree. FerroTERM moves from 0.1.6 to 0.3.1
  on the same index layout, the viewer's memory limit from 1536m to 1024m, and
  `deploy/hosted/sandbox-ferropix-rollout.sh` carries the operator steps on
  both boxes.
