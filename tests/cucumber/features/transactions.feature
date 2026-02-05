Feature: Transaction Creation
  As a user
  I want to create unsigned transactions
  So that I can prepare payments for offline signing

  Scenario: Create simple unsigned transaction
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I create an unsigned transaction with one recipient
    Then the transaction file should be created
    And the transaction should include the recipient
    And the inputs should be locked

  Scenario: Create transaction with multiple recipients
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I create an unsigned transaction with multiple recipients
    Then the transaction should include all recipients
    And the total amount should be correct

  Scenario: Create transaction with payment ID
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I create an unsigned transaction with payment ID "invoice-12345"
    Then the transaction should include the payment ID

  Scenario: Create transaction with insufficient balance
    Given I have a test database with an existing wallet
    And the wallet has zero balance
    When I try to create an unsigned transaction
    Then the transaction creation should fail
    And I should see an insufficient balance error

  Scenario: Create transaction with custom lock duration
    Given I have a test database with an existing wallet
    And the wallet has sufficient balance
    When I create an unsigned transaction with lock duration "3600" seconds
    Then the inputs should be locked for "3600" seconds
