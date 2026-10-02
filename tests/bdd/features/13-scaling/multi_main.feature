@spec-7.3 @phase-4
Feature: Multi-main leader election
  When two or more `r8r start` mains share a queue-mode database
  (`N8N_MULTI_MAIN_SETUP_ENABLED=true`, `EXECUTIONS_MODE=queue`), only the
  elected leader runs schedules, pollers and other long-lived triggers; a
  Redis lock (`SET NX EX`, renewed with a compare-and-expire script, plan
  task 4.1) decides who that is. Webhooks and forms stay registered on
  every main. Losing the lock (e.g. the leader process dies) hands
  leadership to the other main within the lock's TTL plus the check
  interval. Single-main, the default, is always leader and unaffected.

  Requires Redis (R8R_BDD_REDIS_HOST/PORT) and PostgreSQL
  (R8R_BDD_POSTGRES_URL); opt in with
  R8R_BDD_INCLUDE=requires-redis,requires-postgres.

  @requires-redis @requires-postgres
  Scenario: Two mains elect one leader and a schedule fires once per tick
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a workflow named "Shared schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
      | Stamp    | set             |                                                                       |
    And the connections "Schedule -> Stamp"
    And the workflow is active
    Then after 5 seconds the workflow has between 3 and 6 executions

  @requires-redis @requires-postgres
  Scenario: Stopping the leader hands the schedule to the other main within the TTL
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a workflow named "Takeover schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    And within 6 seconds the workflow has at least 2 executions
    When I stop the leader main
    And I remember the executions count of the workflow
    Then within 8 seconds the workflow has new executions

  @requires-redis @requires-postgres
  Scenario: A webhook workflow answers on every main, leader or not
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a workflow named "Shared webhook" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "shared", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                      |
    And the node "Echo" sets the fields:
      """
      {"ok": true}
      """
    And the connections "Webhook -> Echo"
    And the workflow is active
    And the mains have synced
    When I send a POST request to "/webhook/shared" on the server named "main"
    Then the response status is 200
    When I send a POST request to "/webhook/shared" on the server named "b"
    Then the response status is 200

  Scenario: Single-main (the default) still fires schedules
    Given a running r8r server with an owner and an API key
    And a workflow named "Solo schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    Then within 6 seconds the workflow has at least 3 executions
