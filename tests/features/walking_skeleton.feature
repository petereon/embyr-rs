Feature: SDK developer retrieves documents after pointing Firebase at embyr
  As Alex, a developer who has moved away from vendor lock-in,
  I want to change only the host and project credentials in my Firebase configuration,
  So that my app works identically against embyr as it did against Firestore.

  Background:
    Given a project is provisioned with customer-owned storage

  @walking_skeleton @driving_port @us_01 @us_03 @real_io
  Scenario: SDK developer retrieves a document they previously wrote
    Given a document "users/alice" containing greeting="hello" exists in the project
    When a Firebase-compatible client requests the document "users/alice" using the project credentials
    Then the client receives the document with greeting="hello"
    And the document is confirmed to exist
    And the response arrives without errors

  @us_01 @real_io
  Scenario: Server reports healthy status while running
    When a client checks whether the service is available
    Then the service confirms it is ready to accept requests

  @us_01 @error
  Scenario: Client pointed at a non-existent host encounters a connection failure
    Given no embyr server is listening at the configured host
    When a Firebase-compatible client attempts its first document operation
    Then the operation returns a connection failure (not a crash)
    And the failure is identifiable as a transport-level problem
