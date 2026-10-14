-- Remembers which send last held an output locked when that send's inputs were
-- handed back (`unlock_outputs_for_request`).
--
-- The transaction monitor rejects a send it could not get into the mempool and
-- releases its inputs, but the send may still be mined (a node can accept it
-- after all). When the scan then sees a released input spent, this column leads
-- it back to the send, which it marks mined. It has no effect on selection or
-- locking: those look at `status` and `locked_by_request_id` only.

ALTER TABLE outputs ADD COLUMN released_from_request_id TEXT;
