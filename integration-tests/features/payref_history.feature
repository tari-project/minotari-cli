Feature: Payref history fallback after reorg
  As a wallet user
  I want transaction lookups by an old payment reference to keep resolving
  So that receipts issued before a block reorg remain verifiable

  Scenario: Completed transaction lookup falls back to the payref history table
    Given I have a test database with an existing wallet
    And a completed transaction "100" exists for the default account with live payref "live_payref_xyz"
    And the completed transaction "100" has historical payref "stale_payref_abc" recorded
    When I start the daemon on port "9100"
    And I request the completed transaction by payref "live_payref_xyz" via the API on port "9100"
    Then the API should return a completed transaction with id "100"
    When I request the completed transaction by payref "stale_payref_abc" via the API on port "9100"
    Then the API should return a completed transaction with id "100"
    When I request the completed transaction by payref "never_seen_payref" via the API on port "9100"
    Then the API should return a 404 for the completed transaction
