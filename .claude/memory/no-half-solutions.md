---
name: no-half-solutions
description: Build every issue the proper way it asks for, even when that needs a breaking change and a minor version bump; never shape a design to dodge semver; validate the need against its consumer first (owner 2026-11-08)
metadata:
  type: feedback
---

Build what the issue asks for, the proper way. If the proper design adds a field to a public struct or changes a signature, that is a breaking change: bump the minor version (0.x to 0.(x+1)), rename the milestone, and write the upgrade line in the changelog. Never tell a worker to "keep it additive" when that forces a weaker design than the issue specifies, and never accept a PR body that asks for an "owner decision" between the proper way and a shortcut. Choose the proper way.

Before building, validate that what the issue asks for is what is needed. Trace the requirement to its consumer: who calls it, through what interface, and to what end. Confirm the requested shape is the right one and record the check on the issue. If the issue asks for the wrong thing, correct the issue first.

**Why:** owner, 2026-11-08, on FerroAUTH #300, and the follow-up the same day: "you should always check and validate if what we should build is also what we need to build". FerroAUTH #299 asked for a configuration key to switch the onward token cache off; a brief told the worker to keep `ferrohealth-smart` 0.9.5 additive, so it built a method on the client instead. The owner: "we do not want shortcuts or half solutions, we always do the proper way".

**How to apply:** when a brief names a release version, it also says that a breaking change moves the version and that a version is never a reason to narrow the design. Review each PR body for trade-offs of this kind and decide for the proper way. Related: [[no-compat-shims]], [[shared-crate-blockers]].
