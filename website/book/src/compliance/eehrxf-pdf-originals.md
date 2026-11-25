<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# PDF/A originals

The draft implementing act on the European electronic health record exchange
format (EHDS Art. 15(1), Have your say initiative 17512, Ares(2026)10884928 of
2026-11-23) lets a document be exchanged as a PDF attachment beside its
structured form: Art. 3(1)(a) and Annex II name PDF/A-1a or PDF/A-1b, embedded
in HL7 FHIR R4. FerroBRIDGE builds that attachment from a PDF that FerroEHR
stores as a `DV_MULTIMEDIA` value. The exchange holds only when the bytes
FerroEHR returns are the bytes committed: the same length, the same digest
and no re-encoding.

<!-- toc -->

## How FerroEHR stores and returns a PDF

`DV_MULTIMEDIA` (openEHR RM 1.2.0, data types) carries the media in one of
two forms:

- **Inline:** the bytes in `data`, which canonical JSON and canonical XML
  write as base64. FerroEHR returns the same bytes in either format,
  whichever format they were committed in.
- **By reference:** a `uri` to where the original lives. FerroEHR stores the
  reference and returns it unchanged; it does not fetch the document.

In both forms the value keeps its `media_type` (`application/pdf` from
`IANA_media-types`), its `size` in bytes and its `integrity_check` with
`integrity_check_algorithm` (`SHA-256` from
`openehr_integrity_check_algorithms`), so a receiver can check what it gets
against the digest the writer committed. With [S3 multimedia](../beyond-core/s3-multimedia.md)
on, a large inline PDF is kept in the object store under its SHA-256, and a
read with `?expand_multimedia=true` checks the fetched bytes against that
digest before it returns them.

## The round trips

Each test commits a composition holding a one-page PDF/A-1b document, which
veraPDF passes for flavour 1b, then reads it back four ways: the composition
and its `ORIGINAL_VERSION`, each in canonical JSON and canonical XML. Every
read must return the committed bytes with the committed `media_type`, `size`
and `integrity_check`. The tests are in
`app/ferroehr-rest/tests/it/pdf_original_http.rs`.

| Test | Committed as | State |
|---|---|---|
| `an_inline_pdf_a_original_committed_as_json_reads_back_byte_for_byte` | inline, canonical JSON | Byte-identical |
| `an_inline_pdf_a_original_committed_as_xml_reads_back_byte_for_byte` | inline, canonical XML | Byte-identical |

A third test, `a_referenced_pdf_a_original_keeps_its_uri_size_and_integrity_check`,
commits the same document by reference and reads back its `uri`, `size` and
`integrity_check` unchanged, with no inline data added.

The [Annex III metadata](eehrxf-annex-iii.md) of the document that carries the
attachment come from the same composition.
