# Concepts

These chapters explain the ideas you need to use FerroEHR effectively. Four
chapters; the first two read in either order, the last two go deeper:

- **[openEHR primer](openehr-primer.md):** the standard itself: the Reference
  Model, archetypes and templates, compositions, versioning, and AQL, with no
  prior openEHR knowledge assumed. Read this first if the words "archetype" and
  "composition" are new to you.
- **[System architecture](architecture.md):** how *this* server is put
  together and where your data actually lives, so the behaviour you see through
  the API makes sense: why the specification layer is generated, how storage and
  versioning work on PostgreSQL, and how an AQL query becomes SQL.
- **[Storage architecture](storage.md):** the deep-dive on the tables: the
  temporal version table, the decomposed node model with its nested-set
  index, the one-transaction write path, and the cold archival tier, each
  with a diagram.
- **[The AQL engine](aql-engine.md):** the query pipeline from lexer to
  `RESULT_SET`, what each stage refuses, and the design reasons queries are
  fast.

None of them is a prerequisite for [Getting started](../getting-started.md);
you can commit a composition and run a query without them. Come back when you
want to know *why* the API behaves as it does, or before you make one of the
decisions you then live with: a specification generation, a template design, a
query shape.
