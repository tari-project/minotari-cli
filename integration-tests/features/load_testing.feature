Feature: Wallet Load Testing
  As a developer
  I want to load test the wallet under various scenarios
  So that I can measure performance and identify bottlenecks

  # Scenarios adapted from https://github.com/brianp/wallet-performance

  Scenario: Pool payout - rapid sequential payments to many recipients
    Given I have a seed node PoolPayoutNode
    And I have a test database with a full signing wallet
    When I mine 64 blocks on PoolPayoutNode
    And I scan the wallet
    When I send 50 transactions at a constant rate of 5 per second
    And I send a burst of 10 transactions as fast as possible
    And I mine 5 blocks on PoolPayoutNode
    And I scan the wallet
    Then all pool payout transactions should succeed
    And I print the load test results for "pool_payout"

  Scenario: Inbound flood - measure latency detecting incoming transactions
    Given I have a seed node InboundFloodNode
    And I have a test database with a full signing wallet
    When I mine 23 blocks on InboundFloodNode
    And I scan the wallet
    When I send 20 inbound transactions using Poisson distribution at 0.5 per second
    And I mine 5 blocks on InboundFloodNode
    And I measure scan detection time for incoming transactions
    Then all inbound transactions should be detected
    And I print the load test results for "inbound_flood"

  Scenario: Bidirectional - simultaneous send and receive under ramping load
    Given I have a seed node BidirectionalNode
    And I have a test database with a full signing wallet
    When I mine 18 blocks on BidirectionalNode
    And I scan the wallet
    When I send transactions with ramping load from 1 to 5 per minute over 5 steps
    And I mine 5 blocks on BidirectionalNode
    And I scan the wallet
    Then all bidirectional transactions should succeed
    And I print the load test results for "bidirectional"

  Scenario: Fragmentation - UTXO aggregation under extreme fragmentation
    Given I have a seed node FragmentationNode
    And I have a test database with a full signing wallet
    When I mine 27 blocks on FragmentationNode
    And I scan the wallet
    When I fragment UTXOs by sending 20 small transactions of 1000 microTari
    And I mine 5 blocks on FragmentationNode
    And I scan the wallet
    And I send aggregation transactions of increasing size
    And I mine 5 blocks on FragmentationNode
    And I scan the wallet
    Then the aggregation transactions should succeed
    And I print the load test results for "fragmentation"

  Scenario: Lock contention - UTXO locking under rapid sequential access
    Given I have a seed node LockContentionNode
    And I have a test database with a full signing wallet
    When I mine 33 blocks on LockContentionNode
    And I scan the wallet
    When I send a batch of 5 rapid transactions
    And I wait 10 seconds for cooldown
    And I send a batch of 10 rapid transactions
    And I wait 10 seconds for cooldown
    And I send a batch of 15 rapid transactions
    And I mine 5 blocks on LockContentionNode
    And I scan the wallet
    Then I print the load test results for "lock_contention"
