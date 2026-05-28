Feature: SDK developer writes documents through the agent
  Background:
    Given the agent is running with a provisioned project "finops-prod"
    And the agent has an active connection to the project's storage

  @driving_port @us_a02 @real_io
  @skip
  Scenario: Creating a new document stores it with generation one
    Given the document "orders/ord-2026-001" does not exist
    When a caller creates "orders/ord-2026-001" with fields customerId="C-489" and amount=1250
    Then the creation succeeds
    And retrieving "orders/ord-2026-001" returns customerId="C-489" and amount=1250
    And the document record shows it is at generation one

  @driving_port @us_a02 @real_io
  @skip
  Scenario: Updating a document with a field mask preserves unmentioned fields
    Given the document "orders/ord-2026-001" exists with fields customerId="C-489" and amount=1250
    When a caller updates "orders/ord-2026-001" setting only status="shipped"
    Then the update succeeds
    And retrieving "orders/ord-2026-001" returns customerId="C-489" and amount=1250 and status="shipped"
    And the document record shows it is at generation two

  @driving_port @us_a02 @real_io
  @skip
  Scenario: Removing an absent document succeeds without error
    Given the document "orders/ord-2026-001" does not exist
    When a caller removes "orders/ord-2026-001"
    Then the removal succeeds without error

  @driving_port @us_a02 @real_io
  @skip
  Scenario: Removing a document leaves a deletion record
    Given the document "orders/ord-2026-001" exists with fields status="pending"
    When a caller removes "orders/ord-2026-001"
    Then the removal succeeds
    And a deletion record for "orders/ord-2026-001" exists in the project storage
    And retrieving "orders/ord-2026-001" returns not-found

  @driving_port @us_a02 @real_io
  @skip
  Scenario: An increment transform on a field absent from the document treats the starting value as zero
    Given the document "orders/ord-2026-001" exists with no "retryCount" field
    When a caller applies an increment of 1 to the "retryCount" field
    Then the update succeeds
    And retrieving "orders/ord-2026-001" shows retryCount=1

  @driving_port @us_a02 @real_io
  @skip
  Scenario: Creating a document without specifying an identifier generates one automatically
    Given no document exists in the "orders" collection with a generated identifier
    When a caller creates a new document in the "orders" collection with status="pending"
    Then the creation succeeds with a generated document identifier
    And the generated identifier is 20 characters long

  @driving_port @us_a02 @real_io @error
  @skip
  Scenario: Concurrent write attempt on same generation is rejected
    Given the document "orders/ord-2026-001" exists at generation two
    When two callers simultaneously attempt to overwrite the document both asserting generation two
    Then exactly one write succeeds advancing to generation three
    And the other caller receives a precondition-failed response

  @driving_port @us_a02 @real_io @error
  @skip
  Scenario: Updating a document that does not exist returns not-found
    Given the document "orders/ord-2026-999" does not exist
    When a caller updates "orders/ord-2026-999" setting status="shipped"
    Then the caller receives a not-found response
    And no document is created in storage

  @driving_port @us_a02 @real_io @error
  @skip
  Scenario: Creating a document that already exists is rejected
    Given the document "orders/ord-2026-001" already exists with status="pending"
    When a caller attempts to create "orders/ord-2026-001" again
    Then the caller receives an already-exists response
    And the existing document remains unchanged

  @driving_port @us_a02 @real_io @property
  @skip
  Scenario: Document generation advances by exactly one on every successful write
    Given the document "orders/ord-2026-001" is written successfully three times in sequence
    When the generation is read after each write
    Then the generations are 1, 2, and 3 in that order
    And no generation value repeats
    And no generation value decreases
