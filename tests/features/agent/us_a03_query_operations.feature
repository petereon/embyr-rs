Feature: SDK developer queries documents through the agent
  Background:
    Given the agent is running with a provisioned project "finops-prod"
    And the agent has an active connection to the project's storage

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Filtered query returns only matching documents
    Given 5 orders exist: 3 with status="pending" and 2 with status="shipped"
    When a caller queries the "orders" collection for documents where status equals "pending"
    Then the caller receives exactly 3 documents
    And none of the returned documents have status="shipped"

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Collection group query traverses nested collections regardless of parent path
    Given "orders/ord-001/line_items/item-1" exists and "orders/ord-002/line_items/item-2" exists
    When a caller runs a collection group query for all "line_items" documents
    Then both documents are returned
    And documents from both parent paths are included

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Count aggregation returns the correct total
    Given 7 orders exist in the "orders" collection for project "finops-prod"
    When a caller runs a count aggregation over the "orders" collection
    Then the caller receives a count of 7

  @driving_port @us_a03 @real_io @error
  @skip
  Scenario: Query with a malformed field path is rejected before any data is read
    Given the "orders" collection contains documents
    When a caller queries with field path "order..amount" which contains a double dot
    Then the caller receives an invalid-request response
    And no documents are scanned from storage

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Listing documents returns results in pages of up to one hundred
    Given 150 documents exist in the "orders" collection
    When a caller lists the documents in the "orders" collection
    Then the first page contains exactly 100 documents
    And the first response includes a continuation token
    And fetching with that continuation token returns the remaining 50 documents
    And the final page response has no continuation token

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Query excluding a field value omits documents missing that field
    Given 3 orders have status="pending" and 2 orders have no status field at all
    When a caller queries the "orders" collection for documents where status does not equal "shipped"
    Then the 3 orders with status="pending" are returned
    And the 2 orders with no status field are not returned

  @driving_port @us_a03 @real_io @error
  @skip
  Scenario: Query over an empty collection returns no documents and a completion signal
    Given no documents exist in the "invoices" collection
    When a caller queries the "invoices" collection without filters
    Then the caller receives zero documents
    And the caller receives a completion signal with no document payload

  @driving_port @us_a03 @real_io
  @skip
  Scenario: Streaming query response indicates completion at the end
    Given 3 orders exist in the "orders" collection
    When a caller runs a streaming query over the "orders" collection
    Then the caller receives a final message indicating the query is complete
    And the final message carries no document payload
