# Access log details definition: provenance

One ADL 2 archetype, the definition of the `ITEM_TREE` an access log entry
keeps in `ACCESS_LOG_ENTRY.other_details`. The binary carries it
(`ferroehr::access_log::definition::DEFINITIONS`), each boot stores it when no
definition of its id is stored, never over a stored one, and
`deploy/definitions/published.sha256` records its content.

1. `openEHR-EHR-ITEM_TREE.access_log_details.v1.0.0.adls`: the `ITEM_TREE`
   root and one `ELEMENT` per detail, each with the data type the server
   writes it as.

## Source

FerroEHR's own work. The access log model (`ACCESS_LOG_ENTRY`) is a draft,
no openEHR release, and openEHR CKM publishes no archetype of its
`other_details`.

## Our own modelling

- The element node ids are two-digit id-codes, `id11` to `id40`, so their
  byte order is their numeric order: the server and the audit schema order
  the elements of a tree by that byte order.
- An element is written only where the entry's core does not state the fact
  as the access holds it, so a reader reads the core first.
- The terminology is English only.

## Licence

`BUSL-1.1`, Cadasto B.V., recorded in `REUSE.toml`.
