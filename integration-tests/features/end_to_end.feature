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

  Scenario: Node synchronization and wallet scanning
    Given I have a seed node SeedNode
    And I have a base node RegularNode connected to all seed nodes
    And I have a test database with an existing wallet
    When I mine 8 blocks on SeedNode
    Then SeedNode should be at height 8
    And RegularNode should be at height 8
    When I perform a scan with max blocks "20"
    Then the scan should complete successfully
