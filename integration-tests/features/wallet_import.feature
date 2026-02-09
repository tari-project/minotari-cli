Feature: Wallet Import
  As a user
  I want to import an existing wallet
  So that I can restore access to my funds

  Scenario: Import wallet using view and spend keys
    Given I have a test database
    When I import a wallet with view key and spend key
    Then the account should be created in the database
    And the account should have the correct keys

  Scenario: Import wallet with custom birthday
    Given I have a test database
    When I import a wallet with birthday "1000"
    Then the account should have birthday "1000"

  Scenario: Create wallet from seed words
    Given I have a test database
    When I create a wallet with seed words
    Then the account should be created in the database
    And the account should be encrypted with password

  Scenario: Show seed words for existing wallet
    Given I have a test database with an existing wallet
    When I request to show seed words with password
    Then I should see the seed words
