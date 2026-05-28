Feature: Tenant Admin — Cloud secret manager integration
  As Morgan, a DevOps lead managing a cloud-native deployment,
  I want to register my project using an existing AWS/GCP secret for the DB DSN,
  So that embyr uses my existing secret store and password rotation is transparent.

  Background:
    Given an embyr admin API is running and reachable
    And I authenticate with admin_key "operator-key"

  # ── AWS Secrets Manager path ─────────────────────────────────────────────────

  Scenario: Register project with AWS Secrets Manager backend
    Given an AWS secret exists at ARN "arn:aws:secretsmanager:us-east-1:123:secret:my-db-dsn"
    And the secret value is {"dsn": "postgres://user:pass@rds.example.com:5432/prod"}
    And embyr has IAM permission secretsmanager:GetSecretValue on that ARN
    When I POST /admin/v1/projects with:
      | field              | value                                                               |
      | project_id         | morgan-aws                                                          |
      | auth_mode          | static_key                                                          |
      | auth_key           | morgan-secret                                                       |
      | backend_mode       | aws_secret                                                          |
      | backend_secret_arn | arn:aws:secretsmanager:us-east-1:123:secret:my-db-dsn              |
    Then the response status is 201
    And the response body contains project_id "morgan-aws"
    And the embyr system DB does not contain any postgres password
    And migrations are applied to the customer Postgres

  Scenario: AWS password rotation is transparent to the SDK
    Given project "morgan-aws" is active with backend_mode "aws_secret"
    And the AWS secret is rotated to a new password
    When 6 minutes pass (exceeding the 5-minute credential cache TTL)
    And the SDK writes a document
    Then the write succeeds with the new password
    And no application code change is required

  # ── GCP Secret Manager path ──────────────────────────────────────────────────

  Scenario: Register project with GCP Secret Manager backend
    Given a GCP secret exists at "projects/my-gcp-proj/secrets/db-dsn/versions/latest"
    And the secret value is {"dsn": "postgres://user:pass@db.example.com:5432/prod"}
    And embyr's GCP service account has roles/secretmanager.secretAccessor on that secret
    When I POST /admin/v1/projects with:
      | field              | value                                                                  |
      | project_id         | morgan-gcp                                                             |
      | auth_mode          | static_key                                                             |
      | auth_key           | morgan-secret                                                          |
      | backend_mode       | gcp_secret                                                             |
      | backend_secret_gcp | projects/my-gcp-proj/secrets/db-dsn/versions/latest                   |
    Then the response status is 201
    And the embyr system DB does not contain any postgres password
    And migrations are applied to the customer Postgres

  # ── Error Paths ──────────────────────────────────────────────────────────────

  Scenario: Registration fails when embyr lacks IAM access to secret
    Given embyr has no IAM permission on the specified secret ARN
    When I POST /admin/v1/projects with backend_mode "aws_secret"
    Then the response status is 400
    And the error code is "backend_secret_fetch_failed"

  Scenario: Registration fails when secret has wrong format
    Given the AWS secret value is "just-a-plain-string" (not JSON with dsn field)
    When I POST /admin/v1/projects with backend_mode "aws_secret"
    Then the response status is 400
    And the error message mentions "secret format"

  Scenario: Audit confirms only secret reference stored (no plaintext DSN)
    Given project "morgan-aws" is active
    When I inspect the embyr system DB record for project "morgan-aws"
    Then the record contains "backend_secret_arn"
    And the record does not contain any postgres password or connection string
