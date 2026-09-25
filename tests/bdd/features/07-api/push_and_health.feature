@spec-6.9 @spec-8.4 @phase-2
Feature: Live execution push and health endpoints
  The editor follows runs over `/rest/push` (WebSocket, SSE fallback) with
  n8n's message types. Health and metrics endpoints serve orchestrators and
  Prometheus.

  Scenario: A manual run streams its progress to the editor
    Given a running r8r server with an owner and an API key
    And I am connected to the push channel
    And a workflow named "Pushy" with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Step  | noOp          |
    And the connections "Start -> Step"
    When I run the workflow manually from the editor
    Then I receive the push messages in order:
      """
      executionStarted
      nodeExecuteBefore
      nodeExecuteAfter
      nodeExecuteBefore
      nodeExecuteAfter
      executionFinished
      """
    And a "nodeExecuteAfter" push message names the node "Step"

  Scenario: Activating a workflow is announced
    Given a running r8r server with an owner and an API key
    And I am connected to the push channel
    And a workflow named "Announced" with nodes:
      | name    | type    | parameters                                                                        |
      | Webhook | webhook | {"httpMethod": "GET", "path": "announce", "responseMode": "onReceived", "options": {}} |
    When I activate the workflow
    Then I receive the push messages in order:
      """
      workflowActivated
      """

  Scenario: The push endpoint requires a session
    Given a running r8r server with an owner account
    And I am not authenticated
    When I send a GET request to "/rest/push?pushRef=anonymous"
    Then the response status is 401

  Scenario: Liveness
    Given a running r8r server
    When I send a GET request to "/healthz"
    Then the response status is 200
    And the response JSON is:
      """
      {"status": "ok"}
      """

  Scenario: Readiness reports the database connection
    Given a running r8r server
    When I send a GET request to "/healthz/readiness"
    Then the response status is 200
    And the response JSON is:
      """
      {"status": "ok"}
      """

  Scenario: Metrics are off by default
    Given a running r8r server
    When I send a GET request to "/metrics"
    Then the response status is 404

  Scenario: Metrics use n8n's names when enabled
    Given the environment variable "N8N_METRICS" is "true"
    And a running r8r server
    When I send a GET request to "/metrics"
    Then the response status is 200
    And the response header "content-type" contains "text/plain"
    And the response body contains "n8n_version_info"
    And the response body contains "n8n_active_workflow_count"
    And the response body contains "n8n_process_cpu_seconds_total"
