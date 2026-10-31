- **The `openehr-*` crates take their version when they are published.** A
  pull request that changes the crates no longer bumps their lockstep `0.0.x`
  version: the crates publish reads crates.io, takes the next version when
  packaged content changed since the newest published one, and refuses to
  publish changed content under a version crates.io already holds. The
  version in the manifests on `main` can therefore trail the newest version
  on crates.io.
