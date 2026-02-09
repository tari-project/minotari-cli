Feature: Fund Locking
  As a user
  I want to lock funds for pending transactions
  So that I can reserve UTXOs without immediately spending them

  Scenario: Lock funds for a specific amount
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I lock funds for amount "1000000" microTari
    Then the funds should be locked
    And the locked funds file should be created
    And the UTXOs should be marked as locked

  Scenario: Lock funds with multiple outputs
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I lock funds with "3" outputs
    Then "3" UTXOs should be locked

  Scenario: Lock funds with custom duration
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I lock funds with duration "7200" seconds
    Then the UTXOs should be locked for "7200" seconds

  Scenario: Lock funds with insufficient balance
    Given I have a test database with an existing wallet
    And the wallet has zero balance
    When I try to lock funds for amount "1000000" microTari
    Then the fund locking should fail
    And I should see an insufficient balance error

  Scenario: Lock funds with custom fee rate
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I lock funds with fee per gram "10" microTari
    Then the fee calculation should use "10" microTari per gram
