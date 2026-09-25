@spec-8.1 @spec-8.4
Feature: Performance targets and observability
  The performance numbers are the spec's proposed targets (§8.1), to be
  confirmed against an n8n baseline on reference hardware (4 vCPU, 8 GB).
  They are tagged @perf and opt-in: R8R_BDD_INCLUDE=perf.

  @perf @phase-4
  Scenario: Cold start to ready in under a second
    When I start the r8r server
    Then the server accepted connections within 1000 ms of being started

  @perf @phase-4
  Scenario: Idle memory stays small
    Given a running r8r server
    Then after 5 seconds idle the server uses at most 60 MB of resident memory

  @perf @phase-4
  Scenario: An onReceived webhook answers quickly under load
    Given a running r8r server with an owner and an API key
    And a workflow named "Hot path" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "hot", "responseMode": "onReceived", "options": {}}  |
      | Tag     | set     |                                                                                     |
    And the node "Tag" adds the fields:
      """
      {"seen": true}
      """
    And the connections "Webhook -> Tag"
    And the workflow setting "saveDataSuccessExecution" is "none"
    And the workflow is active
    When I send 5000 POST requests to "/webhook/hot" with concurrency 50
    Then every load response had the status 200
    And the p99 latency is at most 15 ms
    And the throughput is at least 500 requests per second

  @perf @phase-4
  Scenario: A Set node over 100k items stays within memory and time budgets
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Make  | code          |
      | Tag   | set           |
    And the node "Make" runs the JavaScript:
      """
      return Array.from({ length: 100000 }, (_, i) => ({ json: { i, a: { b: i * 2 } } }));
      """
    And the node "Tag" adds the fields:
      """
      {"double": "={{ $json.a.b }}"}
      """
    And the connections "Start -> Make -> Tag"
    When I execute the workflow allowing 120 seconds
    Then the execution succeeds
    And the node "Tag" outputs 100000 items
    And the command finished within 20000 ms

  @phase-4
  Scenario: Logs can be structured JSON with execution context
    Given the environment variable "N8N_LOG_FORMAT" is "json"
    And a running r8r server with an owner and an API key
    And a workflow named "Logged" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "GET", "path": "logged", "responseMode": "lastNode", "options": {}}  |
    And the workflow is active
    When I send a GET request to "/webhook/logged"
    Then every server log line is a JSON object with "level" and "message"
    And a server log line has the fields "executionId, workflowId"

  @phase-4
  Scenario: Execution metrics are exported
    Given the environment variable "N8N_METRICS" is "true"
    And a running r8r server with an owner and an API key
    And a workflow named "Counted" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "GET", "path": "counted", "responseMode": "lastNode", "options": {}} |
    And the workflow is active
    When I send a GET request to "/webhook/counted"
    And I send a GET request to "/metrics"
    Then the response body contains "n8n_workflow_executions_total"
