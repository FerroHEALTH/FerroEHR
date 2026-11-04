- The deployment artifacts run the newest FerroHEALTH releases, each pinned
  by its index digest: FerroTERM 0.4.1
  (`ghcr.io/ferrohealth/ferroterm:0.4.1@sha256:b7f10609992377a7b8bca666254bceca79fe4366e2e54fc5fbce079eb9abf619`)
  in the terminology overlay, the SUT stack, the hosted sandbox and the Helm
  chart, with the index layout unchanged; FerroPIX 0.0.15
  (`ghcr.io/ferrohealth/ferropix:0.0.15@sha256:5572f80a3a17fe83228a2a6fcf40f1dcaf8cacc48275136927286ba6ded5c74b`)
  in the compose quickstart, the hosted sandbox and the chart, with the
  FerroPIX chart 0.3.7 built for it; and FerroBRIDGE 0.0.19
  (`ghcr.io/ferrohealth/ferrobridge:0.0.19@sha256:bdf4bd416f772ac36e4b3a940eb67841989552c0a0950d680049d959bd54dcba`)
  in the chart. None of the three renames or removes a configuration key
  FerroEHR sets. The chart moves to 21.0.0-rc13.
