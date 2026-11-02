-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: drop the outbox reader registry (#4766).
--
-- The party outbox, like the clinical one, is drained by the AMQP publisher
-- through `published_at` and read by no cursor reader, so its registry goes
-- with the clinical one and the retention prune decides on the window alone.
-- No openEHR spec governs eventing: our own design/extension.
DROP TABLE event_outbox_reader;
