-- A burn is an outflow: a recovered burn output never credits the balance.
-- Remove what earlier scans booked for burn outputs.

-- The "received" entries built from such a credit: no inputs, and every output
-- a burn output of the account. Output hashes are stored in the JSON as arrays
-- of byte values, so each is turned into hex to compare with the blob column.
DELETE FROM displayed_transactions
WHERE json_array_length(transaction_json, '$.details.inputs') = 0
  AND json_array_length(transaction_json, '$.details.outputs') > 0
  AND NOT EXISTS (
      SELECT 1
      FROM json_each(displayed_transactions.transaction_json, '$.details.outputs') AS o
      WHERE NOT EXISTS (
          SELECT 1
          FROM outputs
          WHERE outputs.account_id = displayed_transactions.account_id
            AND outputs.is_burn = 1
            AND hex(outputs.output_hash) = (
                SELECT group_concat(printf('%02X', b.value), '' ORDER BY b.key)
                FROM json_each(o.value, '$.hash') AS b
            )
      )
  );

-- The credits and their reorg reversals, so the ledger nets correctly.
DELETE FROM balance_changes
WHERE caused_by_output_id IN (SELECT id FROM outputs WHERE is_burn = 1);
