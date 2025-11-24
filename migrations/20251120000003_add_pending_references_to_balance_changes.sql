-- Add references to pending outputs and inputs in balance_changes table
-- These allow tracking balance changes caused by pending (unconfirmed) transactions

ALTER TABLE balance_changes ADD COLUMN caused_by_pending_output_id INTEGER REFERENCES pending_outputs(id);

ALTER TABLE balance_changes ADD COLUMN caused_by_pending_input_id INTEGER REFERENCES pending_inputs(id);
