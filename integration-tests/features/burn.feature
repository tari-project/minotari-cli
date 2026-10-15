Feature: Burn Funds
  As a user
  I want to burn L1 funds to produce a claim proof for L2
  So that I can claim them on the Tari sidechain

  # ─────────────────────────────────────────────────────────────────────────────
  # Error-path scenarios (no base node required — failures happen before broadcast)
  # ─────────────────────────────────────────────────────────────────────────────

  Scenario: Burn fails with insufficient balance
    Given I have a test database with an existing wallet
    And the wallet has zero balance
    When I try to burn "1000000" microTari
    Then the burn command should fail
    And the error output should contain "insufficient funds"

  Scenario: Burn fails with invalid claim public key
    Given I have a test database with an existing wallet
    When I try to burn "1000000" microTari with claim public key "not-valid-hex"
    Then the burn command should fail
    And the error output should contain "claim-public-key"

  # ─────────────────────────────────────────────────────────────────────────────
  # Durability scenario: burn proof is written to the DB *before* broadcast.
  # We point the command at an unreachable URL so broadcast always fails, but
  # the partial proof record must already be in the database by that point.
  # The burn is recorded before the broadcast, so the command does not fail:
  # it reports the burn as pending, for the monitor to rebroadcast.
  # ─────────────────────────────────────────────────────────────────────────────

  Scenario: Burn proof is persisted to database before broadcast attempt
    Given I have a test database with a full signing wallet
    And the wallet has sufficient balance
    When I burn "100000" microTari targeting an unreachable base node
    Then the burn is recorded as pending
    And the database should contain a "pending_merkle" burn proof record
