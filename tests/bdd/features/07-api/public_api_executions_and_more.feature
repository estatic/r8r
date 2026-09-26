@spec-6.9 @phase-2
Feature: Public API: executions, variables, users, audit and docs

  Background:
    Given a running r8r server with an owner and an API key
    And a workflow named "Probe" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "probe", "responseMode": "lastNode", "options": {}}   |
      | Check   | if      |                                                                                     |
    And the node "Check" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ $json.body.ok }}", "rightValue": "", "operator": {"type": "boolean", "operation": "true", "singleValue": true}}],
        "combinator": "and"}, "options": {}}
      """
    And the connections "Webhook -> Check"
    And the workflow is active

  Scenario: Executions are listed with their status and mode
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": true}
      """
    And I wait for the execution to finish
    And I send a GET request to "/api/v1/executions?workflowId=%{WORKFLOW_ID}"
    Then the response status is 200
    And the response JSON at "data" has 1 element
    And the response JSON at "data[0]" matches:
      """
      {"id": "$any", "finished": true, "mode": "webhook", "status": "success", "workflowId": "%{WORKFLOW_ID}", "startedAt": "$datetime", "stoppedAt": "$datetime"}
      """
    And the response JSON at "data[0]" matches:
      """
      {"retryOf": null}
      """

  Scenario: One execution is fetched with its run data
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": true}
      """
    And I wait for the execution to finish
    And I send a GET request to "/api/v1/executions/%{EXECUTION_ID}?includeData=true"
    Then the response status is 200
    And the response JSON at "data.resultData.runData.Check[0]" matches:
      """
      {"startTime": "$number", "executionTime": "$number", "executionStatus": "success", "data": {"main": [[{"json": {"body": {"ok": true}}}], []]}}
      """

  Scenario: Without includeData the run data is omitted
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": true}
      """
    And I wait for the execution to finish
    And I send a GET request to "/api/v1/executions/%{EXECUTION_ID}"
    Then the response status is 200
    And the response JSON has no key "data.resultData"

  Scenario: Executions can be filtered by status
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": "not a boolean"}
      """
    And I wait for the execution to finish
    And I send a GET request to "/api/v1/executions?status=error&workflowId=%{WORKFLOW_ID}"
    Then the response JSON at "data" has 1 element
    When I send a GET request to "/api/v1/executions?status=success&workflowId=%{WORKFLOW_ID}"
    Then the response JSON at "data" has 0 elements

  Scenario: An execution can be deleted
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": true}
      """
    And I wait for the execution to finish
    And I send a DELETE request to "/api/v1/executions/%{EXECUTION_ID}"
    Then the response status is 200
    When I send a GET request to "/api/v1/executions/%{EXECUTION_ID}"
    Then the response status is 404

  Scenario: A failed execution can be retried
    When I send a POST request to "/webhook/probe" with body:
      """
      {"ok": "not a boolean"}
      """
    And I wait for the execution to finish
    And I send a POST request to "/api/v1/executions/%{EXECUTION_ID}/retry"
    Then the response status is 200
    And the response JSON matches:
      """
      {"mode": "retry"}
      """

  @n8n-licensed
  Scenario: Variables can be managed
    When I send a POST request to "/api/v1/variables" with body:
      """
      {"key": "API_BASE", "value": "https://api.example.com"}
      """
    Then the response status is 201
    When I send a GET request to "/api/v1/variables"
    Then the response JSON at "data" contains an element matching:
      """
      {"key": "API_BASE", "value": "https://api.example.com"}
      """

  @n8n-licensed
  Scenario: Variable keys must be valid identifiers
    When I send a POST request to "/api/v1/variables" with body:
      """
      {"key": "not valid!", "value": "x"}
      """
    Then the response status is 400

  Scenario: Users can be listed by the owner
    When I send a GET request to "/api/v1/users?includeRole=true"
    Then the response status is 200
    And the response JSON at "data" contains an element matching:
      """
      {"email": "owner@example.com", "role": "global:owner"}
      """

  Scenario: A security audit can be generated
    When I send a POST request to "/api/v1/audit" with body:
      """
      {}
      """
    Then the response status is 200

  Scenario: The OpenAPI description is served
    Given I am not authenticated
    When I send a GET request to "/api/v1/openapi.yml"
    Then the response status is 200
    And the response body contains "openapi:"
    When I send a GET request to "/api/v1/docs"
    Then the response status is one of "200, 301, 302"
