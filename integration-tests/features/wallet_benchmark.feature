Feature: Wallet Performance Benchmarking
  As a developer
  I want to benchmark wallet performance
  So that I can measure scanning and transaction confirmation times

  @pie
  Scenario: Benchmark wallet scanning and transaction confirmation performance
    Given I have a seed node BenchmarkNode
    And I have a test database with an existing wallet
    When I mine 500 blocks on BenchmarkNode
    Then I measure the time to scan 500 blocks
    Then the scan should complete successfully
    When I check the balance for account "default"
    Then the balance should be at least 1753895088580 microTari
    When I send 10 transactions
    And I mine 5 blocks on BenchmarkNode
    And I measure the time to confirm 10 transactions
    Then 10 transactions of 1000 uT should be confirmed
    Then I print the benchmark results
