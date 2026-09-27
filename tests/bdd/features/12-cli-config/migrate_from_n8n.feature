@spec-2.1 @spec-7.1 @phase-3 @r8r-only
Feature: Migrating an existing n8n instance
  `r8r migrate-from-n8n --db=<n8n database>` imports an n8n 1.x/2.x
  database into r8r's own, keeping n8n's ids: users keep their passwords,
  API keys keep working, credentials stay encrypted with the same
  N8N_ENCRYPTION_KEY, active workflows are active again, and waiting
  executions resume from the URLs n8n handed out. The n8n database is only
  read. The fixture is a real n8n 2.35.7 database.

  Background:
    Given the n8n database "n8n.sqlite" created from the fixture "n8n-2.35-sqlite.sql"
    And the environment variable "N8N_ENCRYPTION_KEY" is "migrate-key"

  Scenario: An imported instance serves its users, API clients and webhooks
    When I run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite"
    Then the command succeeds
    And the command output contains "workflows:    3 (2 active)"
    And the command output contains "executions:   4 (1 waiting)"
    Given a running r8r server
    When I log in as the owner
    Then the response status is 200
    Given the API key stored in the n8n database "n8n.sqlite" is the key of "owner"
    When I use the API key of "owner"
    And I send a GET request to "/api/v1/workflows?active=true"
    Then the response status is 200
    And the response JSON at "data" contains an element matching:
      """
      {"name": "Orders", "active": true, "tags": [{"name": "billing"}]}
      """
    When I send a GET request to "/api/v1/executions?status=error"
    Then the response JSON at "data" has 1 element
    When I am not authenticated
    And I send a POST request to "/webhook/orders" with body:
      """
      {"sku": "M-9", "qty": 4}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"sku": "M-9", "qty": 8}
      """

  Scenario: A waiting n8n execution resumes from the URL n8n issued
    Given I successfully run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite"
    And a running r8r server
    And the API key stored in the n8n database "n8n.sqlite" is the key of "owner"
    When I send a POST request to "/webhook-waiting/4?signature=wrong-token" with body:
      """
      {"approved": true}
      """
    Then the response status is 401
    When I send a POST request to "/webhook-waiting/4?signature=74ccf07f2f93be5b9547db8955005846fdbe49bffc8ec9bb7f610fa38b3a02d6" with body:
      """
      {"approved": true}
      """
    Then the response status is 200
    Given the execution "4" is remembered
    When I wait for that execution to finish
    Then the execution succeeds
    And the node "Decision" outputs:
      """
      [{"approved": true, "request": "laptop"}]
      """

  Scenario: Credentials stay readable with the n8n instance's key
    Given I successfully run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite --skip-executions"
    When I run "r8r export:credentials --all --decrypted --output=plain.json"
    Then the command succeeds
    And the file "plain.json" has the credential "Partner token" with the decrypted data:
      """
      {"name": "X-Token", "value": "partner-secret-1"}
      """

  Scenario: Importing twice updates instead of duplicating
    Given I successfully run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite"
    When I run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite"
    Then the command succeeds
    And the command output contains "executions already in r8r (kept): 4"
    And the command output contains "workflows:    3 (2 active)"

  Scenario: A different encryption key is refused before anything is written
    Given the environment variable "N8N_ENCRYPTION_KEY" is "not-the-n8n-key"
    When I run "r8r migrate-from-n8n --db=sqlite:n8n.sqlite"
    Then the command fails
    And the command output contains "N8N_ENCRYPTION_KEY"

  # Both default to ~/.n8n/database.sqlite, so this is about SQLite.
  Scenario: r8r never opens an n8n database in place
    Given the environment variable "DB_TYPE" is "sqlite"
    And the environment variable "DB_SQLITE_DATABASE" is "n8n.sqlite"
    When I run "r8r export:workflow --all"
    Then the command fails
    And the command output contains "holds an n8n database"
