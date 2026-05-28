Feature: Document write, read, delete, and batch operations
  As Alex (SDK Developer),
  I want to write, read, update, delete, and batch-read documents via the Firestore protocol,
  So that I can build any data-driven feature without changing my application code.

  Background:
    Given a project is provisioned with customer-owned storage
    And the project is active

  # US-02: Write a document

  @us_02 @real_io @driving_port
  Scenario: Writing a document makes it immediately readable
    When a client writes document "products/widget" with name="Widget", price=9.99
    Then reading "products/widget" returns name="Widget" and price=9.99
    And the write result includes a server-assigned timestamp

  @us_02 @real_io
  Scenario: Writing the same document twice updates the stored value
    Given a document "settings/config" exists with theme="light"
    When a client updates "settings/config" setting theme="dark"
    Then reading "settings/config" returns theme="dark"

  @us_02 @error
  Scenario: Two clients writing the same document at the same version — one is rejected
    Given document "counters/visits" exists at version 3
    When two clients simultaneously attempt to update "counters/visits" specifying version 3
    Then exactly one update succeeds and reaches version 4
    And the other update receives an optimistic conflict rejection

  @us_02 @error
  Scenario: Writing to a suspended project is denied
    Given the project is suspended
    When a client attempts to write document "data/x"
    Then the write is rejected with a permission denial
    And no document is stored

  @us_02 @error
  Scenario: Writing with an incorrect credential is rejected
    When a client uses the wrong project credentials to write document "data/y"
    Then the write is rejected with an authentication failure

  # US-03: Read a document

  @us_03 @real_io @driving_port
  Scenario: Reading a document that exists returns its current fields
    Given document "users/bob" exists with role="editor", active=true
    When a client reads "users/bob"
    Then the client receives role="editor" and active=true
    And the document is confirmed to exist

  @us_03 @real_io
  Scenario: All Firestore field types survive a write-read round trip
    When a client writes a document containing a string, a number, a boolean, a timestamp, an array, a map, and a null value
    Then reading the document back returns every field with its original value and type

  @us_03 @error
  Scenario: Reading a document that has never been written returns an absent indicator
    When a client reads "users/nobody"
    Then the client receives an absent-document indicator
    And no error is returned

  # US-06: Delete

  @us_06 @real_io
  Scenario: Deleting a document makes it absent to subsequent reads
    Given document "temp/cache" exists
    When a client deletes "temp/cache"
    Then reading "temp/cache" returns an absent-document indicator
