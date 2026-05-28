Feature: SDK developer receives real-time change notifications through the agent
  Background:
    Given the agent is running with a provisioned project "finops-prod"
    And the agent has an active connection to the project's storage

  @driving_port @us_a05 @real_io
  @skip
  Scenario: Change notification arrives within two seconds of a committed write
    Given a caller has opened a change subscription on the "orders" collection
    When a caller commits a write setting "orders/ord-2026-001" status="shipped"
    Then the subscription caller receives a change event for "orders/ord-2026-001"
    And the change event arrives within 2 seconds of the write committing
    And the change event reflects status="shipped"

  @driving_port @us_a05 @real_io @error
  @skip
  Scenario: Overflow of pending change events triggers a reset notification
    Given 64 pending change events are buffered in the subscription channel for project "finops-prod"
    When one additional document is written to the project
    Then the overflow indicator is set
    And all active subscription callers for "finops-prod" receive a reset-and-resync notification

  @driving_port @us_a05 @real_io @error
  @skip
  Scenario: Subscription stream restores delivery after a connection interruption
    Given a caller has an active change subscription on "finops-prod"
    When the subscription stream is interrupted simulating an agent restart
    Then the subscription caller receives a reset-and-resync notification
    And after the connection is restored new writes are again delivered within 2 seconds

  @driving_port @us_a05 @real_io @property
  @skip
  Scenario: Change event for a written document carries the complete document contents
    Given documents in project "finops-prod" have fields totalling up to 100 kilobytes
    When those documents are written and change events are pushed to active subscribers
    Then each change event carries the complete document fields with no fields missing
    And no field values are truncated

  @driving_port @us_a05 @real_io
  @skip
  Scenario: Change event for an upsert carries a generation of at least one
    Given a document "orders/ord-2026-001" is created in project "finops-prod"
    When the resulting change event is received by an active subscriber
    Then the change event kind is "upsert"
    And the change event generation is at least 1

  @driving_port @us_a05 @real_io
  @skip
  Scenario: Change event for a deleted document carries the delete kind
    Given the document "orders/ord-2026-001" exists in project "finops-prod"
    And a caller has an active change subscription
    When the document "orders/ord-2026-001" is removed
    Then the subscriber receives a change event for "orders/ord-2026-001"
    And the change event kind is "delete"
