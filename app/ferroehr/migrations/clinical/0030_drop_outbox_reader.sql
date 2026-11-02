-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: drop the outbox reader registry (#4766).
--
-- No lane the server ships reads the outbox through a cursor: the AMQP
-- publisher drains it by `published_at`, and the one cursor reader, the
-- outbound FHIR emitter, left with 0013. A registry row nothing advances or
-- reconciles would only hold the retention prune back, so the prune now
-- decides on the window alone and the registry goes. No openEHR spec governs
-- eventing: our own design/extension.
DROP TABLE event_outbox_reader;
