Feature: Blockchain Scanning
  As a user
  I want to scan the blockchain
  So that I can detect incoming transactions

  Scenario: Initial scan from birthday height
    Given I have a test database with an existing wallet
    And the wallet has birthday height "0"
    When I perform a scan with max blocks "10"
    Then the scan should complete successfully
    And the scanned tip should be updated

  Scenario: Incremental scan from last scanned height
    Given I have a test database with an existing wallet
    And the wallet has been previously scanned
    When I perform an incremental scan
    Then the scan should start from the last scanned height
    And new blocks should be processed

  Scenario: Re-scan from specific height
    Given I have a test database with an existing wallet
    And the wallet has been previously scanned to height "100"
    When I re-scan from height "50"
    Then the wallet state should be rolled back to height "50"
    And scanning should resume from height "50"

  Scenario: Scan with custom batch size
    Given I have a test database with an existing wallet
    When I perform a scan with batch size "5"
    Then blocks should be fetched in batches of "5"
