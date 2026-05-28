Feature: SDK developer retrieves documents through the agent
  Background:
    Given the agent is running with a provisioned project "finops-prod"
    And the agent has an active connection to the project's storage

  @walking_skeleton @driving_port @us_a01 @real_io
  @skip
  Scenario: Agent returns document fields to an authenticated caller
    Given the document "users/riley" exists with fields name="Riley" and role="devops"
    When an authenticated caller requests the document "users/riley"
    Then the caller receives the document with name="Riley" and role="devops"
    And the agent audit log records the request with the project identifier and path

  @driving_port @us_a01 @real_io @error
  @skip
  Scenario: Agent signals document not found for absent path
    Given the document "orders/nonexistent" does not exist in the project
    When an authenticated caller requests the document "orders/nonexistent"
    Then the caller receives a not-found response
    And no error is raised on the caller side

  @driving_port @us_a01 @real_io @error
  @skip
  Scenario: Agent rejects request with empty document path
    When an authenticated caller requests a document with an empty path
    Then the caller receives an invalid-request response
    And the response message references the missing path field
