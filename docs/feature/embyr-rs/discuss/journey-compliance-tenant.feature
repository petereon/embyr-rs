Feature: Compliance-first Tenant — Agent mode credential isolation
  As Riley, a CISO at a compliance-first organization,
  I want to deploy the embyr agent in my VPC so my DB credentials never leave my network,
  So that I can use the Firestore SDK and pass security audits without credential egress.

  Background:
    Given an embyr agent binary is deployed in my VPC at "agent.internal:9191"
    And the agent has EMBYR_AGENT_DB_DSN set to "postgres://user:pass@internal-db:5432/prod"
    And the agent is configured with embyr CA cert in EMBYR_AGENT_CA
    And an embyr admin API is running and reachable from my network

  # ── Happy Path ──────────────────────────────────────────────────────────────

  Scenario: Agent starts and connects to Postgres
    When the embyr agent process starts
    Then the agent logs contain "listening on :9191"
    And the agent logs contain "connected to Postgres"

  Scenario: Register project with agent backend
    When I POST /admin/v1/projects with:
      | field                  | value                          |
      | project_id             | riley-corp                     |
      | auth_mode              | static_key                     |
      | auth_key               | riley-secret                   |
      | backend_mode           | agent                          |
      | backend_agent_endpoint | agent.internal:9191            |
      | backend_agent_ca       | <PEM cert of agent CA>         |
    Then the response status is 201
    And the embyr SaaS system DB does not contain any postgres DSN
    And migrations are applied to the customer Postgres via the agent

  Scenario: SDK operations route through agent
    Given project "riley-corp" is registered with agent backend
    When SDK client writes document "users/alice"
    Then the agent logs show an incoming gRPC call
    And the agent logs show a Postgres query
    And the document is written to the customer Postgres

  Scenario: Security audit confirms zero credential egress
    Given project "riley-corp" is registered with agent backend
    When I inspect the embyr SaaS system DB for project "riley-corp"
    Then no DSN or database password is stored
    And only "backend_agent_endpoint" and "backend_agent_ca" are stored

  Scenario: Certificate rotation with zero downtime
    Given project "riley-corp" is registered with agent backend
    When I update the agent certificate and key (EMBYR_AGENT_CERT / EMBYR_AGENT_KEY)
    And I perform a rolling restart of the agent pod
    And I PATCH /admin/v1/projects/riley-corp with the new backend_agent_ca
    Then SDK requests succeed during and after the rotation
    And no "connection refused" errors are observed

  # ── Error Paths ─────────────────────────────────────────────────────────────

  Scenario: Registration fails when agent is unreachable from embyr SaaS
    When I POST /admin/v1/projects with backend_agent_endpoint "unreachable:9191"
    Then the response status is 400
    And the error code is "backend_agent_unreachable"

  Scenario: Agent refuses connection with invalid embyr CA cert
    Given the agent EMBYR_AGENT_CA does not trust embyr's cert
    When embyr SaaS attempts to connect to the agent
    Then the mTLS handshake fails
    And no data is transmitted

  Scenario: Agent cannot start if DSN is missing
    Given EMBYR_AGENT_DB_DSN is not set
    When the agent process starts
    Then the agent exits with a non-zero code
    And the error message mentions "EMBYR_AGENT_DB_DSN"
