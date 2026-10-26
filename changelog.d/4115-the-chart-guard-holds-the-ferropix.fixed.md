- **The chart guard holds the FerroPIX Artifact Hub image to its pinned
  digest** (#4115). `scripts/checks/chart-appversion.sh` checked the
  FerroTERM and FerroBRIDGE `artifacthub.io/images` entries against their
  pins, but not the FerroPIX entry the `ferropix` dependency added, so its
  digest could drift from `ferropix.image.digest` in `values.yaml` and
  Artifact Hub would scan an image the chart does not run. The guard now
  compares the two, and `--self-test` moves the digest on either side.
