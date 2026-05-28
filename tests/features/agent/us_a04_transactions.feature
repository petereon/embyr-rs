Feature: SDK developer uses transactions through the agent
  Background:
    Given the agent is running with a provisioned project "finops-prod"
    And the agent has an active connection to the project's storage

  @driving_port @us_a04 @real_io
  @skip
  Scenario: Transaction with no concurrent competition commits successfully
    Given the document "orders/ord-2026-001" exists at generation 3 with status="processing"
    When a caller opens a transaction, reads "orders/ord-2026-001", and commits with status="complete"
    Then the transaction commits successfully
    And retrieving "orders/ord-2026-001" shows status="complete" at generation 4

  @driving_port @us_a04 @real_io @error
  @skip
  Scenario: Concurrent transaction on same generation is rejected with a conflict
    Given the document "orders/ord-2026-001" exists at generation 3
    And two transactions both open and read "orders/ord-2026-001" at generation 3
    When both transactions attempt to commit
    Then exactly one commit succeeds advancing the document to generation 4
    And the other caller receives a conflict-aborted response citing "orders/ord-2026-001"

  @driving_port @us_a04 @real_io @error
  @skip
  Scenario: Transaction reading a document that was later deleted is aborted on commit
    Given the document "orders/ord-2026-001" exists at generation 5
    And a transaction opens and reads "orders/ord-2026-001"
    And another caller deletes "orders/ord-2026-001" before the transaction commits
    When the transaction attempts to commit
    Then the commit returns an aborted response indicating "orders/ord-2026-001" was removed

  @driving_port @us_a04 @real_io @error
  @skip
  Scenario: Committing an expired transaction returns not-found
    Given a transaction was opened but its time-to-live has elapsed
    When a caller attempts to commit that transaction
    Then the caller receives a not-found response for the transaction

  @driving_port @us_a04 @real_io
  @skip
  Scenario: Rolling back a transaction discards writes without applying them
    Given the document "orders/ord-2026-001" exists at generation 2 with status="processing"
    And a transaction opens and prepares a write setting status="complete"
    When the caller rolls back the transaction
    Then retrieving "orders/ord-2026-001" still shows status="processing" at generation 2

  @driving_port @us_a04 @real_io @error
  @skip
  Scenario: Rolling back a transaction that has already been committed returns not-found
    Given a transaction that has already committed successfully
    When a caller attempts to roll back that same transaction
    Then the caller receives a not-found response for the transaction
    And the committed document changes remain intact

  @driving_port @us_a04 @real_io
  @skip
  Scenario: Expired transactions are removed by the sweep operation
    Given two transactions exist: one expired and one still active
    When the sweep operation runs
    Then the expired transaction record is removed
    And the active transaction record remains
