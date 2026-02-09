Feature: Base Node Integration
  As a developer
  I want to test wallet operations against a real base node
  So that I can verify actual blockchain interactions

  Scenario: Start a base node
    Given I have a seed node Node_A
    Then the base node should be running
