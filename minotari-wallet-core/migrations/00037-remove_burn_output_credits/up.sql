-- A burn is an outflow: a recovered burn output never credits the balance.
-- Remove what earlier scans booked for burn outputs.

-- Burn outputs stored before `is_burn` existed (00029) still have it 0. A
-- serialized `OutputType` is its number, and Burn is 2. A row whose JSON is not
-- valid is left alone rather than failing the migration.
UPDATE outputs
SET is_burn = 1
WHERE is_burn = 0
  AND CASE WHEN json_valid(wallet_output_json)
           THEN json_extract(wallet_output_json, '$.features.output_type')
      END = 2;

-- A burn output can never be spent. Remove any spend recorded for one (its
-- input and the debits and reversals tied to that input) and make it unspent
-- again, as a scan stores it.
DELETE FROM balance_changes
WHERE caused_by_input_id IN (
    SELECT inputs.id FROM inputs JOIN outputs ON outputs.id = inputs.output_id WHERE outputs.is_burn = 1
);

DELETE FROM inputs
WHERE output_id IN (SELECT id FROM outputs WHERE is_burn = 1);

UPDATE outputs
SET status = 'UNSPENT'
WHERE is_burn = 1 AND status IN ('SPENT_UNCONFIRMED', 'SPENT');

-- The "received" entries built from such a credit: no inputs, and every output
-- a burn output of the account. Output hashes are stored in the JSON as arrays
-- of byte values, so each is turned into hex and looked up in the set of burn
-- outputs, built once (the subquery is not correlated). A hash that is not a
-- byte array becomes '' and so never counts as a burn output.
DELETE FROM displayed_transactions
WHERE EXISTS (SELECT 1 FROM outputs WHERE is_burn = 1)
  AND json_array_length(transaction_json, '$.details.inputs') = 0
  AND json_array_length(transaction_json, '$.details.outputs') > 0
  AND NOT EXISTS (
      SELECT 1
      FROM json_each(displayed_transactions.transaction_json, '$.details.outputs') AS o
      WHERE displayed_transactions.account_id || ':' || COALESCE(
                (SELECT group_concat(printf('%02X', b.value), '' ORDER BY b.key) FROM json_each(o.value, '$.hash') AS b),
                ''
            ) NOT IN (SELECT account_id || ':' || hex(output_hash) FROM outputs WHERE is_burn = 1)
  );

-- The credits and their reorg reversals, so the ledger nets correctly.
DELETE FROM balance_changes
WHERE caused_by_output_id IN (SELECT id FROM outputs WHERE is_burn = 1);
