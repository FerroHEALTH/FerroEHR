- **The chart pins the FerroPIX release beside its digest.**
  `ferropix.image.tag` (`0.0.7`) states the release whose image
  `ferropix.image.digest` pins, and `scripts/checks/chart-appversion.sh` now
  holds the tag in the chart's Artifact Hub images entry to it, as it holds
  the digest. The dependency chart still deploys the digest, so the rendered
  manifests do not change.
