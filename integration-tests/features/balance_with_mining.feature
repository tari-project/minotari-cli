Feature: Balance Verification with Mining
  As a user
  I want to verify exact balances after mining
  So that I can confirm the wallet correctly tracks blockchain rewards

  Scenario: Verify balance after mining single block
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 1 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 1000000 microTari

  Scenario: Verify balance after mining multiple blocks
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 5 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 5000000 microTari

  Scenario: Verify balance increases with incremental mining
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 3 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 3000000 microTari
    When I mine 2 blocks on MinerNode
    And I perform an incremental scan
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 5000000 microTari
