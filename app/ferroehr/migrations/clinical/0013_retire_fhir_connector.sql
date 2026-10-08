-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: retire the in-tree FHIR connector's stores.
--
-- FHIR R4 resource mapping left this product for FerroBRIDGE
-- (https://github.com/FerroHEALTH/FerroBRIDGE), which reaches the CDR over
-- ITS-REST and keeps its mappings in its own store. `fhir_mapping`, the
-- mapping registry of the inbound facade, served only the connector; the file
-- that created it is in a release and stays as it is, and this file drops the
-- table. A stored mapping does not carry over to FerroBRIDGE, whose mappings
-- are FHIRconnect documents.
--
-- The outbound FHIR emitter left with it. Its cursor row in
-- `event_outbox_reader` is deleted here, because a reader the server no longer
-- runs or reconciles at boot would otherwise hold the outbox retention prune
-- floor for good. The registry and the outbox stay.
--
-- No openEHR spec governs FHIR interop or eventing: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

DROP TABLE fhir_mapping;

DELETE FROM event_outbox_reader WHERE reader = 'fhir-outbound';
