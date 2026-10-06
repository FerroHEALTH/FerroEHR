# Introduction

FerroEHR is a pure-Rust [openEHR](https://www.openehr.org/) Clinical Data
Repository (CDR): a headless, API-first server that stores and queries
structured health records through a vendor-neutral REST API and the Archetype
Query Language. This book is the user-facing guide — how to run it, configure
it, talk to its API, query it, and load the templates that give your data
shape. If you build clinical applications, operate healthcare infrastructure,
or are evaluating an openEHR back end, you are in the right place.

## What openEHR gives you

openEHR separates *clinical knowledge* from *software*. The structure and
meaning of clinical data — a blood-pressure reading, a medication order, a
discharge summary — live in shared, computable models called archetypes and
templates, authored by clinicians and modellers rather than baked into
application code. Applications then store and retrieve that data through a
standard API, against a shared Reference Model, so the same record is portable
across every conformant system.

FerroEHR implements that standard natively. It speaks the openEHR **REST API**
(ITS-REST Release-1.1.0), executes **Archetype Query Language (AQL 1.1)**, and
holds data as canonical openEHR compositions with full, indelible version
history. There is no proprietary data format in the middle: what you commit is
what you query and what you read back.

## What makes this implementation different

- **Conformance you can run yourself.** The openEHR conformance catalogue is
  executed by a committed runner against a live server, over both canonical
  JSON and canonical XML, and the profile verdicts are computed from the
  per-case outcomes. The run records live in the repository, and every number
  on the [Conformance](conformance.md) page is derived from them at build time,
  never hand-typed.
- **The openEHR specifications, generated** directly from the official
  machine-readable models: the Reference Model, the Archetype Model (1.4 and
  2.4), the serialization schemas, and the REST API contract (Release-1.1.0).
  openEHR's own published terminology (3.1) ships embedded, byte-identical to
  upstream. A specification update is a regeneration.
- **Two selectable specification generations.** One configuration key,
  `spec_profile`, chooses the whole generation set the server runs:
  `development` (Reference Model 1.2.0 with BASE 1.3.0 and LANG 1.1.0 — the
  default) or `stable` (the latest *released* generations, Reference Model
  1.1.0 with BASE 1.2.0 and LANG 1.0.0). See
  [`spec_profile`](installation/configuration.md#spec_profile).
- **One self-contained binary.** No JVM, no language runtime, and a pure-Rust
  TLS stack, so there is nothing to provision beside PostgreSQL. The container
  image is shell-less and runs as non-root.
- **PostgreSQL 18-native storage.** Clinical documents are decomposed into an
  indexed node model with time-bounded, versioned rows; canonical openEHR JSON
  is stored verbatim so storage and API never disagree.

## How the system is layered

FerroEHR is built in two layers. A **specification layer** is generated
deterministically from openEHR's published models: the Reference Model types,
canonical JSON/XML serialization, the REST contract, and the AQL front end. On
top of it sits the **application**: the server, the PostgreSQL-native storage,
the AQL execution engine, validation, and security. The
[System architecture](concepts/architecture.md) chapter walks through this in
user terms; if you are new to openEHR itself, start with the
[openEHR primer](concepts/openehr-primer.md).

## Where to go next

- **Wondering why this exists?** [Why FerroEHR exists](why-ferroehr.md) is the
  project's position: what openEHR is worth, what we commit to, what the
  licence lets you do, how building a product on it works, and why
  contributing back pays.
- **Just want to try it?** [Getting started](getting-started.md) takes you from
  `docker compose up` to a stored composition and an AQL result in a few
  minutes.
- **Deploying it?** [Installation](installation/index.md) covers Docker
  Compose, Kubernetes/Helm, building from source, and the
  [configuration reference](installation/configuration.md);
  [Operations](operations.md) covers running it afterwards.
- **Integrating an application?** [Using the API](using-the-api/index.md) and
  [Querying with AQL](querying-aql.md) are the core reference for client
  developers.
- **Modelling clinical data?** [Templates & validation](templates-validation.md)
  explains how templates drive what the server will accept.
- **Reviewing it for a deployment?** [Security](security.md)
  and the [Threat model](threat-model.md) state what is enforced and what is
  yours to enforce; the [FerroEHR Viewer](viewer/index.md) is the optional web
  UI over the same public API.

> [!NOTE]
> FerroEHR began as a fork of **EHRbase** (by vitasystems and the Peter L.
> Reichertz Institute) and records that lineage in the labelled import commit
> at the root of its history, but it is an
> independent, from-scratch Rust implementation with no EHRbase code in this
> tree, and it is not affiliated with or endorsed by the EHRbase project.
> FerroEHR's own code is source-available under the Business Source License
> 1.1; vendored openEHR material keeps its upstream terms: Apache-2.0 for the
> machine-readable artifacts, CC-BY-SA 3.0 for the specification text, and
> CC-BY-SA 3.0 and 4.0 for the clinical models (see
> [Licensing & legal](licensing.md)).
