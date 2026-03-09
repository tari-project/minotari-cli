Feature: End-to-End Wallet Testing
  As a user
  I want to perform complete wallet operations
  So that I can verify the entire workflow with real blockchain data

  Scenario: Mine, scan, and check balance
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 10 blocks on MinerNode
    Then the chain height should be 10
    When I perform a scan with max blocks "20"
    Then the scan should complete successfully
    And the scanned tip should be updated
    When I check the balance for account "default"
    Then I should see the balance information
    And the balance should be at least 10000000 microTari

  Scenario: Multi-block mining with incremental scanning
    Given I have a seed node MinerNode
    And I have a test database with an existing wallet
    When I mine 5 blocks on MinerNode
    And I perform a scan with max blocks "10"
    Then the scan should complete successfully
    When I mine 3 blocks on MinerNode
    And I perform an incremental scan
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then I should see the balance information
