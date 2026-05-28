Feature: Project provisioning, monitoring, and lifecycle management
  As Sam (Service Operator),
  I want to create, inspect, suspend, and delete projects via the management interface,
  So that I can onboard customers, enforce SLAs, and maintain service quality without
  without manual customer data setup.

  Background:
    Given the service is running with operator-level credentials configured

  # US-07: Provision a project

  @us_07 @real_io @driving_port
  Scenario: Operator provisions a new customer project
    When the operator creates a project named "acme" pointing to acme's own data store
    Then the project is created successfully
    And the operator receives a one-time credential for the customer
    And the credential is not stored in the operator's infrastructure in plain form
    And the customer's data store has the required document storage structure applied

  @us_07 @real_io @kpi
  Scenario: Provisioning a project completes within 5 seconds
    When the operator creates a project named "fast-co" pointing to a reachable data store
    Then the project is created within 5 seconds

  @us_07 @error
  Scenario: Project name with uppercase letters is rejected
    When the operator attempts to create a project named "InvalidName"
    Then the request is rejected with an invalid-name error

  @us_07 @error
  Scenario: Creating a project with an already-used name is rejected
    Given a project named "existing-co" already exists
    When the operator attempts to create another project named "existing-co"
    Then the request is rejected indicating the name is already taken

  @us_07 @error
  Scenario: Pointing a project at an unreachable database fails gracefully
    When the operator attempts to create a project pointing at a data store that is not reachable
    Then the request is rejected with a storage-unavailable error
    And no partial project record is left behind

  @us_07 @error
  Scenario: Management request without operator credentials is rejected
    When a management request is sent without providing operator credentials
    Then the request is rejected with an authentication failure

  @us_07 @error
  Scenario: Management request with wrong operator credentials is rejected
    When a management request is sent with incorrect operator credentials
    Then the request is rejected with an authentication failure

  @us_07 @error
  Scenario: Management interface is not reachable on the customer data channel
    When an attempt is made to reach the management interface via the customer data channel
    Then no management response is returned on the data channel

  # US-08: Monitor project

  @us_08 @real_io @driving_port
  Scenario: Operator retrieves project status and mode
    Given a project named "active-co" exists with direct-storage connectivity
    When the operator retrieves the project details for "active-co"
    Then the response shows the project is active with direct-storage connectivity
    And the response contains no credentials, keys, or connection strings

  @us_08 @real_io @kpi
  Scenario: Usage data is recorded after customer activity
    Given a project named "busy-co" exists
    And the customer has made at least one data request
    When the operator queries today's usage for "busy-co"
    Then usage data is present for today's date

  @us_08 @error
  Scenario: Requesting details of a project that does not exist returns not-found
    When the operator retrieves project details for "no-such-project"
    Then the response indicates the project was not found

  @us_08 @error
  Scenario: Requesting details of a deleted project returns not-found
    Given a project that has been removed
    When the operator retrieves that project's details
    Then the response indicates the project was not found

  # US-09: Suspend and lifecycle

  @us_09 @real_io @driving_port
  Scenario: Suspending a project blocks all customer data requests within 1 second
    Given a project named "overdue-co" is active and customer data requests succeed
    When the operator suspends "overdue-co"
    Then customer data requests for "overdue-co" are rejected with a permission failure
    And the rejection takes effect within 1 second of the suspend action

  @us_09 @real_io
  Scenario: Reactivating a suspended project restores customer data access
    Given a project is suspended
    When the operator reactivates the project
    Then customer data requests for that project succeed again

  @us_09 @error
  Scenario: Suspending an already-suspended project succeeds without error
    Given a project is already suspended
    When the operator suspends it again
    Then the action succeeds (no conflict error)

  @us_09 @real_io
  Scenario: Removing a project makes it immediately unfindable
    Given a project named "leaving-co" exists
    When the operator removes "leaving-co"
    Then retrieving "leaving-co" returns not-found immediately
    And customer data requests for "leaving-co" return not-found

  @us_09 @error
  Scenario: Customer data requests for a removed project return not-found
    Given a project has been removed
    When a customer attempts a data operation using that project's credentials
    Then the request is rejected indicating the project was not found
