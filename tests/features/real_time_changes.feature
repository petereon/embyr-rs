Feature: Real-time document change notifications
  As Alex (SDK Developer),
  I want to subscribe to live document changes and receive updates within 2 seconds,
  So that I can build collaborative, reactive features without a separate pub-sub service.

  Background:
    Given a project is provisioned with customer-owned storage
    And the project is active

  # US-05a: initial snapshot

  @us_05 @real_io @driving_port
  Scenario: Subscribing to a collection delivers all existing documents immediately
    Given 3 documents exist in collection "messages"
    When a client opens a real-time subscription for "messages"
    Then the client receives all 3 documents as an initial snapshot
    And the client receives a signal indicating the snapshot is complete

  # US-05b: CURRENT marker ordering

  @us_05 @real_io
  Scenario: The snapshot-complete signal arrives after the last document, not before
    Given 5 documents exist in collection "events"
    When a client opens a real-time subscription for "events"
    Then the client receives the snapshot-complete signal only after all 5 documents have arrived

  # US-05c: live push (KPI)

  @us_05 @real_io @kpi
  Scenario: A document written by another client appears in the subscription within 2 seconds
    Given a client has an active real-time subscription for collection "scores"
    When a second client writes a new document to "scores"
    Then the subscribing client receives a change notification within 2 seconds

  # US-05d: delete propagation

  @us_05 @real_io
  Scenario: Deleting a document sends a removal notification to active subscribers
    Given document "chat/msg1" exists and a client is subscribed to collection "chat"
    When another client deletes "chat/msg1"
    Then the subscribing client receives a removal notification for "chat/msg1"

  # US-05e: resume token delta delivery

  @us_05 @real_io
  Scenario: Reconnecting with a recent subscription token delivers only changes since disconnect
    Given a client had an active subscription and received a subscription-position token
    And 3 documents were written while the client was disconnected for 30 seconds
    When the client reconnects using the saved subscription-position token
    Then only the 3 new change notifications are delivered
    And the full initial snapshot is not re-sent

  # US-05f: stale resume token

  @us_05 @real_io @error
  Scenario: A subscription-position token older than 24 hours triggers a fresh snapshot without error
    Given a subscription-position token encoding a time 25 hours ago
    When a client opens a subscription presenting that outdated token
    Then the client receives a full fresh snapshot of all current documents
    And no error is returned to the client
    And the client eventually receives a new current subscription-position token

  # Buffer overflow / RESET

  @us_05 @error
  Scenario: A client that cannot keep up with incoming changes receives a resync instruction
    Given a real-time subscription has accumulated more pending changes than the system can buffer
    When the next document change arrives
    Then the client receives a resync instruction
    And the client can re-establish its subscription to receive a fresh snapshot

  # Idle keep-alive

  @us_05 @real_io
  Scenario: An idle subscription receives a keep-alive signal after 30 seconds of no changes
    Given a real-time subscription is active with no pending writes
    When 30 seconds pass with no document changes
    Then the client receives a keep-alive signal

  # US-04f: filtered subscription rejects un-indexed query

  @us_04 @us_05 @error
  Scenario: Subscribing with a complex filter that requires an index fails without one
    Given no composite index exists for the subscription's filter
    When a client opens a real-time subscription with a multi-field filter and sort order
    Then the subscription is rejected with a missing-index error
    And the error message guides the operator to create the required index

  # Property: latency KPI

  @us_05 @property @kpi
  Property: Write-to-notification latency stays within 2 seconds for any number of concurrent subscribers
    Given a project has up to 100 concurrent real-time subscriptions active
    When writes occur at a steady rate across the subscribed collections
    Then 99 out of every 100 change notifications arrive within 2 seconds of the triggering write
