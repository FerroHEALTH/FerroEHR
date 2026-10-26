- **The book states FerroEHR's contract as a federation node** (#3383).
  The new page "FerroEHR as a federation node" reads the openEHR Federation
  Tier with AQL 1.0.0 from the node's side. It lists the endpoints a gateway
  uses and resolution by the local `ehr_id` alone. It gives the four
  admission conditions of N42a and the configuration that meets them: a
  unique `system_id`, and an import under an `ehr_id` the node chose. It also
  covers authentication by the deployment's own authorization server and the
  refusals the node applies whatever the gateway did. It names FerroFED 0.0.13
  as the release whose harness runs FerroEHR as its node. An integration test
  moves an EHR between two nodes under a node-chosen id and finds every
  version uid and its `creating_system_id` unchanged.
