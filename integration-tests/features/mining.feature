Feature: Mining and Blockchain
  As a developer
  I want to mine blocks and verify blockchain state
  So that I can test wallet operations with real blockchain data

  Scenario: Mine blocks on a base node
    Given I have a seed node MinerNode
    When I mine 5 blocks on MinerNode
    Then the chain height should be 5

  Scenario: Mine additional blocks
    Given I have a seed node MinerNode
    When I mine 3 blocks on MinerNode
    And I mine 2 blocks on MinerNode
    Then the chain height should be 5

  Scenario: Sync between two nodes
    Given I have a seed node SeedNode
    And I have a base node RegularNode connected to all seed nodes
    When I mine 10 blocks on SeedNode
    Then SeedNode should be at height 10
    And RegularNode should be at height 10
