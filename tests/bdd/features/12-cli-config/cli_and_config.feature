@spec-5.1 @spec-4.1 @spec-3.2
Feature: One binary, typed configuration
  r8r ships as one binary whose role is picked by subcommand. Configuration
  uses n8n's variable names where they still make sense (goal G4), is typed
  and validated at boot, and can be checked with `r8r config check`.

  @phase-1 @r8r-only
  Scenario: The CLI lists its subcommands
    When I run "r8r --help"
    Then the command succeeds
    And the command output contains "start"
    And the command output contains "worker"
    And the command output contains "webhook"
    And the command output contains "execute"
    And the command output contains "import:workflow"
    And the command output contains "export:workflow"
    And the command output contains "import:credentials"
    And the command output contains "export:credentials"
    And the command output contains "migrate-from-n8n"
    And the command output contains "config"

  @phase-1
  Scenario: The CLI reports its version
    When I run "r8r --version"
    Then the command succeeds
    And the command output contains "r8r"

  @phase-1
  Scenario: An unknown subcommand fails with usage help
    When I run "r8r frobnicate"
    Then the command fails
    And the command output contains "Usage"

  @phase-1
  Scenario: execute reports an unknown workflow id clearly
    When I run "r8r execute --id=does-not-exist"
    Then the command fails
    And the command output contains "does-not-exist"

  @phase-1
  Scenario: execute runs an imported workflow by id
    Given the file "wf.json" contains:
      """
      {"id": "wf-by-id", "name": "By id", "nodes": [
        {"parameters": {}, "id": "1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0]}
      ], "connections": {}, "settings": {}}
      """
    And I successfully run "r8r import:workflow --input=wf.json"
    When I run "r8r execute --id=wf-by-id --rawOutput"
    Then the command succeeds
    And the execution succeeds

  @phase-1 @r8r-only
  Scenario: config check validates and prints the effective configuration
    Given the environment variable "N8N_PORT" is "5999"
    When I run "r8r config check"
    Then the command succeeds
    And the command output contains "N8N_PORT"
    And the command output contains "5999"

  @phase-1 @r8r-only
  Scenario: config check never prints secrets
    When I run "r8r config check"
    Then the command succeeds
    And the command output does not contain "bdd-n8n-encryption-key"

  @phase-1 @r8r-only
  Scenario Outline: Invalid values are rejected at boot, naming the variable
    Given the environment variable "<variable>" is "<value>"
    When I run "r8r config check"
    Then the command fails
    And the command output contains "<variable>"

    Examples:
      | variable                  | value     |
      | N8N_PORT                  | not-a-port |
      | EXECUTIONS_MODE           | sideways  |
      | DB_TYPE                   | oracle    |
      | EXECUTIONS_DATA_MAX_AGE   | -5        |
      | N8N_LOG_LEVEL             | chatty    |

  @phase-2
  Scenario: The server listens on port 5678 by default
    Given the environment variable "N8N_PORT" is not set
    When I start the r8r server
    And I send a GET request to "/healthz"
    Then the response status is 200

  # n8n 2.35 starts anyway; the spec requires boot-time validation.
  @phase-2 @beyond-n8n
  Scenario: The server refuses to start with an invalid configuration
    Given the environment variable "EXECUTIONS_MODE" is "sideways"
    When I start the r8r server
    Then the server fails to start with a message containing "EXECUTIONS_MODE"

  @phase-2
  Scenario: A missing encryption key is generated once and then reused
    Given the environment variable "N8N_ENCRYPTION_KEY" is not set
    And the credential "Kept" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Key", "value": "kept-value"}
      """
    When I run "r8r export:credentials --all --decrypted --output=plain.json"
    Then the command succeeds
    And the file "plain.json" has the credential "Kept" with the decrypted data:
      """
      {"name": "X-Key", "value": "kept-value"}
      """

  @phase-3 @r8r-only
  Scenario: migrate-from-n8n refuses an unsupported database
    Given the file "not-a-db.sqlite" contains:
      """
      this is not a database
      """
    When I run "r8r migrate-from-n8n --db=sqlite://not-a-db.sqlite"
    Then the command fails
    And the command output contains "not-a-db.sqlite"
