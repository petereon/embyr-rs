Feature: SDK Developer — Firebase SDK compatibility with embyr
  As Alex, an SDK developer with an existing Firebase app,
  I want to point my app at an embyr endpoint and have it behave identically to Google Firestore,
  So that I can eliminate vendor lock-in without changing application code.

  Background:
    Given an embyr server is running at "localhost:8081"
    And a project exists with id "my-proj" and auth_key "test-key"

  # ── Happy Path ──────────────────────────────────────────────────────────────

  Scenario: Configure SDK to use embyr endpoint
    When I call firebase.initializeApp with apiKey "test-key" and host "localhost:8081"
    And I call getFirestore()
    Then no error is thrown
    And the Firestore instance points to "localhost:8081"

  Scenario: Write a document
    Given the SDK is configured for project "my-proj"
    When I call setDoc for path "users/alice" with data {name: "Alice", age: 30}
    Then the promise resolves without error
    And the document "users/alice" exists in the customer database

  Scenario: Read a document that was written
    Given the document "users/alice" exists with data {name: "Alice", age: 30}
    When I call getDoc for path "users/alice"
    Then snapshot.exists() is true
    And snapshot.data() equals {name: "Alice", age: 30}

  Scenario: Query a collection with filter and ordering
    Given documents exist in collection "users":
      | path         | name    | age |
      | users/alice  | Alice   | 30  |
      | users/bob    | Bob     | 17  |
      | users/carol  | Carol   | 25  |
    And a composite index exists on "users" for fields [age ASC]
    When I query collection "users" where age >= 18 orderBy age
    Then the result contains 2 documents
    And the documents are ordered by age ascending
    And "users/bob" is not in the result

  Scenario: Listen for real-time updates via onSnapshot
    Given the SDK is configured for project "my-proj"
    When I call onSnapshot on collection "users"
    Then the initial snapshot is delivered immediately
    When a second client writes document "users/dave" with data {name: "Dave", age: 22}
    Then the onSnapshot listener fires within 2 seconds
    And the snapshot includes "users/dave"

  Scenario: Run a transaction with optimistic concurrency
    Given the document "counters/hits" exists with data {count: 0}
    When I run a transaction that reads and increments count
    Then the transaction commits
    And "counters/hits" has count 1

  Scenario: Reconnect with resume token after network drop
    Given an onSnapshot listener is active on collection "users"
    And the listener has received a resume token
    When the network drops for 30 seconds
    And the document "users/eve" is written during the disconnect
    And the SDK reconnects
    Then only the delta (users/eve added) is delivered
    And no full re-snapshot is sent

  # ── Error Paths ─────────────────────────────────────────────────────────────

  Scenario: SDK configured with wrong host hangs on first operation
    Given the SDK is configured with host "wrong-host:9999"
    When I call getDoc for path "users/alice" with a 5 second timeout
    Then the operation times out
    And no response is received

  Scenario: Write fails with permission-denied for wrong auth key
    Given the SDK is configured with auth_key "wrong-key"
    When I call setDoc for path "users/alice" with data {name: "Alice"}
    Then the operation fails with error code "permission-denied"

  Scenario: Query without required index returns FailedPrecondition
    Given no composite index exists on "users" for fields [age ASC, name ASC]
    When I query collection "users" where age >= 18 orderBy age orderBy name
    Then the operation fails with error code "failed-precondition"
    And the error message mentions "index"

  Scenario: Transaction aborted under concurrent writes is retried by SDK
    Given the document "counters/hits" exists with data {count: 0}
    When two clients simultaneously run a transaction incrementing count
    Then one transaction commits with count 1
    And the other transaction is retried and commits with count 2
    And no data is lost

  Scenario: Expired resume token triggers full re-snapshot
    Given an onSnapshot listener holds a resume token older than 24 hours
    When the SDK reconnects
    Then a full snapshot is delivered
    And no error is returned to the caller
