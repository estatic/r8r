@spec-7.3 @spec-8.3 @phase-4
Feature: Queue mode with workers
  In queue mode the main process and webhook processors enqueue executions
  and workers run them. Jobs are leased with heartbeats, so a job whose
  worker dies is picked up again instead of being lost.

  Requires Redis (R8R_BDD_REDIS_HOST/PORT) or PostgreSQL
  (R8R_BDD_POSTGRES_URL); opt in with R8R_BDD_INCLUDE=requires-redis or
  requires-postgres.

  @requires-redis
  Scenario: A worker runs executions enqueued by main
    Given queue mode backed by Redis
    And a running r8r server with an owner and an API key
    And a running r8r worker named "w1"
    And a workflow named "Queued" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "queued", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                     |
    And the node "Echo" sets the fields:
      """
      {"n": "={{ $json.body.n }}"}
      """
    And the connections "Webhook -> Echo"
    And the workflow is active
    When I send a POST request to "/webhook/queued" with body:
      """
      {"n": 5}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"n": 5}
      """

  @requires-redis
  Scenario: Executions wait in the queue until a worker is available
    Given queue mode backed by Redis
    And a running r8r server with an owner and an API key
    And a workflow named "Later" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "later", "responseMode": "onReceived", "options": {}} |
    And the workflow is active
    When I send a POST request to "/webhook/later" with body:
      """
      {}
      """
    Then the response status is 200
    When I start an r8r worker named "late"
    And I wait for the execution to finish
    Then the execution succeeds

  @requires-redis @slow
  Scenario: A job whose worker dies is re-queued after its lease expires
    Given queue mode backed by Redis
    And a running r8r server with an owner and an API key
    And a running r8r worker named "doomed"
    And a workflow named "Survivor" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "POST", "path": "survivor", "responseMode": "onReceived", "options": {}} |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 5, "unit": "seconds"}                             |
      | Done    | noOp    |                                                                                        |
    And the connections "Webhook -> Pause -> Done"
    And the workflow setting "saveExecutionProgress" is true
    And the workflow is active
    When I send a POST request to "/webhook/survivor" with body:
      """
      {}
      """
    And I wait for an execution with the status "running"
    And the worker "doomed" is killed
    And I start an r8r worker named "rescuer"
    And I wait for the execution to finish
    Then the execution status is one of "success, crashed"

  @requires-postgres
  Scenario: The PostgreSQL queue backend runs executions without Redis
    Given queue mode backed by PostgreSQL
    And a running r8r server with an owner and an API key
    And a running r8r worker named "pg-worker"
    And a workflow named "PG queued" with nodes:
      | name    | type    | parameters                                                                            |
      | Webhook | webhook | {"httpMethod": "POST", "path": "pg-queued", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                       |
    And the node "Echo" sets the fields:
      """
      {"ok": true}
      """
    And the connections "Webhook -> Echo"
    And the workflow is active
    When I send a POST request to "/webhook/pg-queued" with body:
      """
      {}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"ok": true}
      """
