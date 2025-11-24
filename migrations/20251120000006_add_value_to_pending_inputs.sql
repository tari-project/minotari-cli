-- Add value column to pending_inputs table
-- This stores the value of the output being spent

ALTER TABLE pending_inputs ADD COLUMN value INTEGER NOT NULL DEFAULT 0;
