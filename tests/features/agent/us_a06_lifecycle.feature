Feature: Operations team deploys and shuts down the agent safely
  # No Background — lifecycle tests control agent startup and shutdown directly

  @driving_port @us_a06 @real_io
  @skip
  Scenario: Agent logs storage readiness before accepting caller connections
    Given all required configuration is set and the storage backend is reachable
    When the agent process starts
    Then the log records a storage-connected message before a port-ready message
    And a document retrieval call to the agent succeeds immediately after both lines appear

  @driving_port @us_a06 @real_io @error
  @skip
  Scenario: Agent exits without binding a port when storage is unreachable
    Given the storage address points to an unreachable host
    When the agent process starts
    Then the log contains a connection-failure message
    And the process exits with a non-zero exit code
    And no listening port is bound on :9191

  @driving_port @us_a06 @real_io
  @skip
  Scenario: Agent completes in-flight work before exiting on shutdown signal
    Given the agent is running and has an in-flight document retrieval in progress
    When a shutdown signal is sent to the agent process
    Then new caller connections are immediately rejected
    And the in-flight retrieval completes and returns its response to the caller
    And the process exits with code 0
    And the log contains "shutdown complete"

  @driving_port @us_a06 @real_io
  @skip
  Scenario: Storage credential never appears in agent logs
    Given the storage connection string contains the sentinel value "DO-NOT-LOG"
    When the agent starts and handles 10 document retrieval calls
    Then no log line at any level contains the text "DO-NOT-LOG"
    And all log output is structured and machine-readable

  @driving_port @us_a06 @real_io @error
  @skip
  Scenario: Agent exits immediately when required storage configuration is absent
    Given the storage connection string configuration is not provided
    When the agent process starts
    Then the process exits with code 1
    And the standard error output names the missing configuration item
    And no listening port is bound on :9191

  @driving_port @us_a06 @real_io @error
  @skip
  Scenario: Agent exits immediately when required project identifier is absent
    Given the project identifier configuration is not provided
    When the agent process starts
    Then the process exits with code 1
    And the standard error output names the missing configuration item
    And no listening port is bound on :9191
