# Beyond the core

The core of FerroEHR is the openEHR platform: EHRs, compositions,
contributions, templates, versioning, and AQL. Around that core the server
carries capabilities for fitting into the systems around it: moving whole
records between systems, storing the people those records refer to, validating
codes against an external terminology server, telling downstream systems that
something changed, and keeping large attachments out of the database.

This chapter set describes each one the way you meet it: what it does, whether
you have to turn it on, and how to consume it.

> [!NOTE]
> FHIR R4 resource mapping is not part of FerroEHR. It is
> [FerroBRIDGE](https://github.com/FerroHEALTH/FerroBRIDGE) (BUSL-1.1), a
> separate server that maps between FHIR and openEHR with FHIRconnect mappings
> and reaches FerroEHR over the ITS-REST API. FerroEHR keeps two uses of FHIR:
> the access log rendered as FHIR `AuditEvent` with the ITI-81 retrieval
> ([Audit](../audit.md)), and the client for external FHIR terminology servers
> ([Terminology servers](terminology.md)).

## What is on already, and what you switch on

Two different things are collected here, and they behave differently:

- **Always mounted, part of the API surface.** The demographic API and the
  `/message` group (EHR Extract and TDD import) are ordinary routes. They have
  no feature switch: they are served on every deployment and gated only by the
  same authentication and authorization as the clinical API
  ([Security](../security.md)). The bundled openEHR terminology
  is likewise always present in-process, with no configuration and no external
  dependency.
- **Off until you configure them.** Everything that reaches *outside* the
  server is opt-in: external terminology servers, change events, and
  multimedia offload to object storage. A bare server contacts none of them, and its
  clinical behaviour is that of a single-tenant, integration-free openEHR CDR
  until you enable one.

> [!IMPORTANT]
> Some of these carry PHI, and each chapter says which. The one that moves
> clinical content off this system is **multimedia offload** (the blob bytes
> land in your bucket). Change-event envelopes carry identifiers and metadata
> only. Treat enabling offload as a deliberate, auditable decision about where
> clinical data is allowed to go.

Every configuration key these chapters name lives in the
[configuration reference](../installation/configuration.md); the integration
sections are on
[Integrations](../installation/config-integrations.md) and
[Audit](../installation/config-audit.md).

## Build features, and what a slim build refuses

Three of these capabilities are also **cargo features** of the server build
(`fhir`, `events`, and `multimedia`) all on in the published binaries and
container images, and on in any default build. Their code lives in a separate
crate the platform pulls in only when the matching feature is on, so a
`--no-default-features` build contains none of it.

A slim build does not start up quietly missing a capability: it **refuses at
boot** when the configuration asks for one it was built without. The `fhir`
feature covers the two uses of FHIR the server keeps: the external FHIR
terminology providers, and the FHIR `AuditEvent` the audit store, the audit
feed and the ITI-81 retrieval carry. See
[From source → Build features](../installation/from-source.md#build-features)
for the exact list of settings a slim binary rejects.

> [!NOTE]
> One gap worth knowing if you build slim: `terminology.api_enabled` is a
> *route* switch, and in a slim build those routes are simply not compiled in,
> so the setting has no effect rather than failing loudly. The boot refusals cover the settings that would otherwise
> lose or fail to deliver data.

## The capability set

- **[EHR Extract & messaging](messaging.md):** export a whole EHR, clone it
  into another system with its distributed version identity intact, import an
  extract into an existing record, and import Template Data Documents (TDDs) as
  compositions.
- **[Demographics](demographics.md):** a versioned party store (persons,
  organisations, groups, agents, roles) with relationships, over a REST surface
  that mirrors the EHR APIs.
- **[Terminology servers](terminology.md):** the bundled openEHR terminology
  for local codes, plus any number of external FHIR terminology servers for
  validating coded values against external value sets; FerroTERM ships with the
  quickstart as the terminology overlay and runs beside the sandbox CDR.
- **[Change events (AMQP)](amqp.md):** a transactional outbox that publishes a
  PHI-free, at-least-once event for every commit to an AMQP broker, so
  downstream systems can respond to changes instead of polling.
- **[S3 multimedia](s3-multimedia.md):** threshold-based, content-addressed
  offload of large `DV_MULTIMEDIA` blobs to any S3-compatible object store, with
  integrity verification on the way back in.

Security, instance separation and the audit trail are covered in
[Security](../security.md); running the server in production,
including the health and observability surfaces these integrations feed, is
covered in [Operations](../operations.md).
