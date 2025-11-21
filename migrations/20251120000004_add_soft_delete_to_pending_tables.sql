-- Add soft delete support to pending_outputs and pending_inputs tables

ALTER TABLE pending_outputs ADD COLUMN deleted_at TIMESTAMP;

ALTER TABLE pending_inputs ADD COLUMN deleted_at TIMESTAMP;
