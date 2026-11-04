- The deployment artifacts run the newest release of each FerroHEALTH
  product, each pinned by its index digest. The terminology overlay, the SUT
  stack, the hosted sandbox and the Helm chart run FerroTERM 0.4.0
  (`ghcr.io/ferrohealth/ferroterm:0.4.0@sha256:efaa80152721c3738e0d0c0695cadbfefe89c1cf6df3567b9d3bc1788767e047`),
  whose index layout is unchanged, so a built index serves as it is. The
  compose quickstart, the hosted sandbox and the chart run FerroPIX 0.0.14
  (`ghcr.io/ferrohealth/ferropix:0.0.14@sha256:389f40d144fe2ef3d79c1c1628e9a662a058961b1650c5541e601addc27112d8`),
  with the FerroPIX chart 0.3.6 built for it. The chart runs FerroBRIDGE
  0.0.17
  (`ghcr.io/ferrohealth/ferrobridge:0.0.17@sha256:4cdf8d4b4c4d626b99a3929a363198067f87ac70875f4ce6db1d0614884f12af`)
  where it ran 0.0.5, and now writes FerroBRIDGE's `[usage_report]` from
  `bridge.usageReport.enabled`. FerroBRIDGE 0.0.17 checks the callers of its
  FHIRconnect operations, FHIR facade and operator API through `[auth]`, and
  refuses to start them until `[auth]` names `issuer` and `audience` or sets
  `enabled = false`. Upgrade: a values file that sets `bridge.mappings` adds
  `bridge.config.auth.issuer` and `bridge.config.auth.audience`, or
  `bridge.config.auth.enabled: false` to serve the operations to every peer
  `bridge.networkPolicy.ingressFrom` admits. The chart moves to
  21.0.0-rc12.
- The viewer draws with FerroUI 0.6.6 (`ferrohealth-ui`), and serves its
  stylesheet and scripts under `/ferrohealth-ui/0.6.6/`, with FerroUI's fixes
  since 0.6.0: a mapping row's relationship words and the top bar's search
  submit fit a 320 px screen, and a combobox is not spell-checked.
