# Introduction

FerroEHR is an [openEHR](https://www.openehr.org/) Clinical Data Repository
(CDR) written in Rust. It is a headless server: it stores structured health
records in PostgreSQL and serves them through the openEHR REST API (ITS-REST
Release-1.1.0) and the Archetype Query Language (AQL 1.1). What you commit is
canonical openEHR, and every version is kept.

The book has four kinds of page: tutorials to learn by doing, how-to guides for
one task each, reference to look things up, and explanation to understand how
the parts fit. Start from what you are here to do.

| You are | Start with | Then |
|---|---|---|
| **An integrator** building on the API | [Getting started](getting-started.md) | [Using the API](using-the-api/index.md), [Querying with AQL](querying-aql.md), [Templates & validation](templates-validation.md), [Problem types](using-the-api/problems.md) |
| **A clinician-facing app builder** launching apps for patients and professionals | [SMART App Launch](smart-app-launch.md) | [How FerroEHR decides access](explanation/access-model.md), [Behind FerroSMART](behind-ferrosmart.md), [The xShare Yellow Button](xshare-yellow-button.md) |
| **An operator** running it in production | [Installation](installation/index.md) | [Configuration reference](installation/configuration.md), [Operations](operations.md), [Go-live checklist](security/go-live-checklist.md), [Operational policy](operational-policy.md) |
| **A decision maker** evaluating it | [Why FerroEHR exists](why-ferroehr.md) | [System architecture](concepts/architecture.md), [How FerroEHR decides access](explanation/access-model.md), [Conformance](conformance.md), [Compliance overview](compliance/index.md) |

New to openEHR? Read the [openEHR primer](concepts/openehr-primer.md) first.
The [FerroEHR Viewer](viewer/index.md) is an optional web UI over the same
public API. Licensing, lineage and trademarks are on
[Licensing & legal](licensing.md).

The name comes from *ferrum*, Latin for iron. Say it "FER-ro-E-H-R".
