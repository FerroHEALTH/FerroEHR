- The hosted sandbox's configuration (`deploy/hosted/ferroehr.sandbox.toml`)
  declares its SCIM service provider under `[auth.scim]` and
  `[[auth.scim.client]]`, the tables 5.7.0 reads, so the sandbox starts on
  5.7.0. Every server configuration the repository ships beside the
  quickstart and the chart (the hosted sandbox's and each compose overlay's)
  and every viewer configuration (the hosted sandbox's, the quickstart's
  with each overlay, and the chart's viewer Deployment) is now read by the
  program built from the change under test in CI, so a shipped file the
  release refuses fails its pull request.
