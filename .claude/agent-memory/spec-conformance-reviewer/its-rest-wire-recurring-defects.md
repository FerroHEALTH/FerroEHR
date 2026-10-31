---
name: its-rest-wire-recurring-defects
description: Confirmed recurring ITS-REST wire defects in ferroehr-rest/ferroehr (item-tag identity, query_parameters form, stored-query name keying, OBJECT_VERSION_ID leniency) with spec citations
metadata:
  type: project
---

Confirmed 2026-07-26 by first-hand spec reading; re-verify before citing (code moves).

- **ITEM_TAG identity is `key` + `target_path` PAIR** (ITS-REST overview
  `Requests_and_responses.md` §"openehr-item-tag and openehr-version-item-tag";
  repeated in `operations/composition_tags_update.yaml`). Our store keys tags by
  `key` alone (`migrations/ehr/0001_baseline.sql` `uq_item_tag_ehr_target_key
  UNIQUE (ehr_id, target_vo_id, key)`; `storage/tag_repo.rs` `last_by_key`
  dedupe; `delete_tag` WHERE key only) → two same-key different-path tags
  silently collapse. Also: `ITEM_TAG.target` per RM
  (`RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`) is a
  **UID_BASED_ID** and "may be a VERSIONED_OBJECT<T> **or a VERSION<T>**"; we
  emit an OBJECT_REF wrapper (OAS shape) and `parse_uid_based_id` drops the
  version part, so VERSION-level tags retarget to the container.
- **Stored-query GET parameters are NAMED, not a `query_parameters` object**
  (`ITS-REST/specifications/docs/query/Request.md` §Query parameters: "in the
  real request they will have specific names", two worked `?uid=…`,
  `?temperature_from=36` examples). The generated params model a literal
  `query_parameters` key (`BTreeMap`, JSON-object literal via
  `overview/params.rs` `deserialize_map`) — the documented named form binds
  nothing.
- **Unqualified stored-query name keying is split**: the wire store path keys
  `('' , name)` (`split_qualified` directly) while `query_exists`/`query_delete`/
  `delete_stored_query_version` key `('misc', name)`
  (`parse_qualified_name(..).qualified()`, our own `misc` default for a bare
  name). Symptom: a query PUT
  under a bare name is undeletable via admin/native calls and a natively registered bare name is
  invisible to the wire GET/list.
- **`creating_system_id` is dropped everywhere** an `OBJECT_VERSION_ID` addresses
  a resource (`version_components`, `components`, `parse_uid_based_id`), and
  `delete_composition` compares only the `TreeId` — a uid identifying no VERSION
  is accepted. `update_composition`/`ehr_status` compare the FULL uid string
  (`service/ehr/mod.rs::ensure_if_match`) but **case-sensitively**, contra BASE
  `base_types/master05-identification_package.adoc` §Composite Identifiers and
  Case ("case-insensitive — two identifiers identical apart from case … identify
  the same thing").
- **ETag/Last-Modified asymmetry in the versioned families**: the `…/version`
  (at-time) reads go through `negotiate::read_rm` (weak ETag set), while
  `…/version/{version_uid}`, `versioned_composition`, `versioned_ehr_status` and
  `revision_history` use `respond_rm` (no ETag) — matches the stalled OAS but not
  the docs text §"ETag and Last-Modified" ("SHOULD be included in responses for
  VERSION, VERSIONED_OBJECT, or other resources that have versioning").
- **Release-1.1.0 tag routes are `/tags`, NOT `/item_tag`** (`ehr.openapi.yaml`
  paths + operation ids `composition_tags_get` etc.). A client written against
  `/item_tag` is on a dev-branch surface.
- Internal-doc citations "review doc 03 req N.N" survive in
  `app/ferroehr/migrations/ehr/0001_baseline.sql` (9 sites) and
  `migrations/ext/0001_openehr_functions.sql` — scrub on touch.
