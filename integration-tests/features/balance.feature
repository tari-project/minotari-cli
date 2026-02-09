Feature: Balance Operations
  As a user
  I want to check my wallet balance
  So that I know how much Tari I have

  Scenario: Check balance for a specific account
    Given I have a test database with an existing wallet
    When I check the balance for account "default"
    Then I should see the balance information
    And the balance should be displayed in microTari

  Scenario: Check balance for all accounts
    Given I have a test database with multiple accounts
    When I check the balance without specifying an account
    Then I should see balance for all accounts

  Scenario: Check balance with no outputs
    Given I have a test database with a new wallet
    When I check the balance for the new wallet
    Then the balance should be zero
