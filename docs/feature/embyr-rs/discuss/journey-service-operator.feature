Feature: Service Operator — Provision and manage tenant projects
  As Sam, a service operator,
  I want to provision projects via the admin API and manage their lifecycle,
  So that I can onboard customers in minutes and enforce SLAs without touching their data.

  Background:
    Given an embyr admin API is running at "localhost:9090"
    And I authenticate with admin_key "operator-key"

  # ── Happy Path ──────────────────────────────────────────────────────────────

  Scenario: Provision a new project with direct Postgres backend
    When I POST /admin/v1/projects with:
      | field           | value                         |
      | project_id      | acme-corp                     |
      | auth_mode       | static_key                    |
      | auth_key        | customer-secret               |
      | backend_mode    | direct_pg                     |
      | backend_pg_dsn  | postgres://user:pass@db:5432/acme |
    Then the response status is 201
    And the response body contains project_id "acme-corp"
    And the response body does not contain "dsn"
    And the response body does not contain "auth_key"
    And the customer database at "postgres://user:pass@db:5432/acme" has migrations applied

  Scenario: Verify project is active after provisioning
    Given project "acme-corp" has been provisioned
    When I GET /admin/v1/projects/acme-corp
    Then the response status is 200
    And the response body contains status "active"
    And the response body contains backend_mode "direct_pg"

  Scenario: Monitor project usage metrics
    Given project "acme-corp" has been active for 2 days
    And the project has received 1000 bytes of ingress today
    When I query daily_project_metrics for project "acme-corp"
    Then I receive a row for today with ingress_bytes >= 1000

  Scenario: Suspend a non-paying project
    Given project "acme-corp" is active
    When I POST /admin/v1/projects/acme-corp/suspend
    Then the response status is 200
    And subsequent SDK requests to project "acme-corp" fail with "permission-denied"
    And the error message contains "project suspended"

  Scenario: Delete a project
    Given project "acme-corp" exists (active or suspended)
    When I DELETE /admin/v1/projects/acme-corp
    Then the response status is 200
    And GET /admin/v1/projects/acme-corp returns 404 immediately
    And a background sweeper is scheduled to purge data after deletion_retention

  # ── Error Paths ─────────────────────────────────────────────────────────────

  Scenario: Provision fails when customer DB is unreachable
    When I POST /admin/v1/projects with backend_pg_dsn "postgres://bad-host:5432/x"
    Then the response status is 400
    And the error code is "backend_unavailable"

  Scenario: Provision rejected for invalid project_id format
    When I POST /admin/v1/projects with project_id "INVALID SPACES"
    Then the response status is 400
    And the error message mentions "project_id"

  Scenario: Provision rejected for duplicate project_id
    Given project "acme-corp" already exists
    When I POST /admin/v1/projects with project_id "acme-corp"
    Then the response status is 409
    And the error code is "already_exists"

  Scenario: Admin API rejects request without valid admin_key
    Given I send no Authorization header
    When I POST /admin/v1/projects with any body
    Then the response status is 401

  Scenario: Suspend a project that is already suspended is idempotent
    Given project "acme-corp" is already suspended
    When I POST /admin/v1/projects/acme-corp/suspend
    Then the response status is 200
    And the project remains suspended
