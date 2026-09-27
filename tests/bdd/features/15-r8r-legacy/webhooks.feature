@r8r-only @legacy-api
Feature: Webhook triggers
  An active workflow whose start node is a webhook runs when its URL is
  called. By default the caller waits for the result; "respond immediately"
  answers 202 at once.

  Background:
    Given a running r8r server
    And I am logged in to the legacy r8r API as "hook@example.com"

  Scenario: An active webhook runs the workflow and returns the result
    Given I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "hook", "nodes": [
        {"id": "hook", "node_type": "core.webhook", "position": [0, 0], "parameters": {"path": "greet", "method": "POST"}},
        {"id": "set1", "node_type": "core.set", "position": [0, 100], "parameters": {"fields": {"received": "{{ $json.body.name }}"}}}
      ], "connections": [{"from_node": "hook", "from_output": 0, "to_node": "set1", "to_input": 0}]}
      """
    And I remember the response JSON at "id" as "wf"
    And I send a PATCH request to "/rest/r8r/workflows/%{wf}/active" with body:
      """
      {"active": true}
      """
    When I send a POST request to "/webhook-r8r/%{wf}/greet" with body:
      """
      {"name": "Ada"}
      """
    Then the response status is 200
    And the response JSON at "status" is "Success"
    And the response JSON at "node_outputs.set1[0].json.received" is "Ada"

  Scenario: Respond immediately answers 202 with the execution id
    Given I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "fast", "nodes": [
        {"id": "hook", "node_type": "core.webhook", "position": [0, 0], "parameters": {"path": "fast", "method": "POST", "respond": "immediately"}}
      ], "connections": []}
      """
    And I remember the response JSON at "id" as "wf"
    And I send a PATCH request to "/rest/r8r/workflows/%{wf}/active" with body:
      """
      {"active": true}
      """
    When I send a POST request to "/webhook-r8r/%{wf}/fast" with body:
      """
      {}
      """
    Then the response status is 202
    And I remember the response JSON at "execution_id" as "exec"
    When I wait for the legacy execution "exec" to finish
    Then the response JSON at "status" is "Success"

  Scenario: An inactive workflow's webhook is not served
    Given I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "off", "nodes": [{"id": "hook", "node_type": "core.webhook", "position": [0, 0], "parameters": {"path": "off", "method": "POST"}}], "connections": []}
      """
    And I remember the response JSON at "id" as "wf"
    When I send a POST request to "/webhook-r8r/%{wf}/off" with body:
      """
      {}
      """
    Then the response status is 404

  Scenario Outline: Calls that don't match the webhook are 404
    Given I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "strict", "nodes": [{"id": "hook", "node_type": "core.webhook", "position": [0, 0], "parameters": {"path": "strict", "method": "POST"}}], "connections": []}
      """
    And I remember the response JSON at "id" as "wf"
    And I send a PATCH request to "/rest/r8r/workflows/%{wf}/active" with body:
      """
      {"active": true}
      """
    When I send a <method> request to "/webhook-r8r/%{wf}/<path>"
    Then the response status is 404

    Examples:
      | method | path   |
      | GET    | strict |
      | POST   | other  |

  Scenario: A webhook whose credential is missing answers 500 before running
    Given I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "hook-cred", "nodes": [
        {"id": "hook", "node_type": "core.webhook", "position": [0, 0], "parameters": {"path": "hc", "method": "POST"}},
        {"id": "h", "node_type": "core.httpRequest", "position": [1, 0], "parameters": {"url": "https://x", "auth": {"type": "bearer", "credential_id": "00000000-0000-0000-0000-000000000002"}}}
      ], "connections": [{"from_node": "hook", "from_output": 0, "to_node": "h", "to_input": 0}]}
      """
    And I remember the response JSON at "id" as "wf"
    And I send a PATCH request to "/rest/r8r/workflows/%{wf}/active" with body:
      """
      {"active": true}
      """
    When I send a POST request to "/webhook-r8r/%{wf}/hc" with body:
      """
      {}
      """
    Then the response status is 500
    And the response body contains "credential resolution failed"
