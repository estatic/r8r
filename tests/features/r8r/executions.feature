@r8r
Feature: Running workflows
  Manual executions run in the background: the API answers 202 with a
  Running execution, which later finishes as Success or Error. Node
  settings (retry, timeout, continue on fail) and error connections decide
  what a failing node does.

  Background:
    Given I am logged in as "run@example.com"

  Scenario: A manual run finishes with each node's output
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "greet", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "set1", "node_type": "core.set", "position": [0, 100], "parameters": {"fields": {"greeting": "hi"}}}
      ], "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "set1", "to_input": 0}]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    Then the response status is 202
    And the JSON at "/status" is "Running"
    And I remember the JSON at "/id" as "exec"
    When I wait for execution "exec" to finish
    Then the JSON at "/status" is "Success"
    And the JSON at "/node_outputs/set1/0/json/greeting" is "hi"
    And the JSON at "/mode" is "Manual"

  Scenario: Expressions read the previous node's data
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "expr", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "a", "node_type": "core.set", "position": [0, 100], "parameters": {"fields": {"n": 20}}},
        {"id": "b", "node_type": "core.set", "position": [0, 200], "parameters": {"fields": {"doubled": "{{ $json.n * 2 }}", "wf": "{{ $workflow.name }}"}}}
      ], "connections": [
        {"from_node": "trigger", "from_output": 0, "to_node": "a", "to_input": 0},
        {"from_node": "a", "from_output": 0, "to_node": "b", "to_input": 0}
      ]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Success"
    And the JSON at "/node_outputs/b/0/json/doubled" is 40
    And the JSON at "/node_outputs/b/0/json/wf" is "expr"

  Scenario: A failing node fails the run
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "boom", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "code1", "node_type": "core.code", "position": [0, 100], "parameters": {"script": "throw new Error('boom')"}}
      ], "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "code1", "to_input": 0}]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Error"

  Scenario: Retries are reported when every attempt fails
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "retry", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "code1", "node_type": "core.code", "position": [0, 100], "parameters": {"script": "throw new Error('boom')"},
         "settings": {"retry": {"max_tries": 3, "wait_ms": 0}}}
      ], "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "code1", "to_input": 0}]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Error"
    And the response body contains "failed after 3 attempts"

  Scenario: Continue on fail passes the error to the next node
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "cof", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "code1", "node_type": "core.code", "position": [0, 100], "parameters": {"script": "throw new Error('boom')"},
         "settings": {"continue_on_fail": true}},
        {"id": "after", "node_type": "core.noop", "position": [0, 200], "parameters": {}}
      ], "connections": [
        {"from_node": "trigger", "from_output": 0, "to_node": "code1", "to_input": 0},
        {"from_node": "code1", "from_output": 0, "to_node": "after", "to_input": 0}
      ]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Success"
    And the response body contains "node execution failed"

  Scenario: An error connection routes the failure to a handler
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "route", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "code1", "node_type": "core.code", "position": [0, 100], "parameters": {"script": "throw new Error('boom')"}},
        {"id": "handler", "node_type": "core.set", "position": [100, 200], "parameters": {"fields": {"handled": true}}}
      ], "connections": [
        {"from_node": "trigger", "from_output": 0, "to_node": "code1", "to_input": 0},
        {"from_node": "code1", "from_output": 0, "to_node": "handler", "to_input": 0, "error": true}
      ]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Success"
    And the JSON at "/node_outputs/handler/0/json/handled" is true

  Scenario: A disabled node passes its input through
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "disabled", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "a", "node_type": "core.set", "position": [0, 100], "parameters": {"fields": {"keep": 1}}},
        {"id": "off", "node_type": "core.set", "position": [0, 200], "parameters": {"fields": {"skipped": true}}, "disabled": true}
      ], "connections": [
        {"from_node": "trigger", "from_output": 0, "to_node": "a", "to_input": 0},
        {"from_node": "a", "from_output": 0, "to_node": "off", "to_input": 0}
      ]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Success"
    And the JSON at "/node_outputs/off/0/json" equals:
      """
      {"keep": 1}
      """

  Scenario: Executing a workflow that does not exist
    When I send a POST request to "/rest/workflows/00000000-0000-0000-0000-000000000000/execute"
    Then the response status is 404

  Scenario: Executing with a credential that does not exist is refused before running
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "ghost-cred", "nodes": [
        {"id": "h", "node_type": "core.httpRequest", "position": [0, 0], "parameters": {"url": "https://example.com", "auth": {"type": "bearer", "credential_id": "00000000-0000-0000-0000-000000000001"}}}
      ], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    Then the response status is 400
    And the response body contains "credential resolution failed"

  Scenario: Past executions are listed newest first
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "history", "nodes": [{"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}}], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    And I sent a POST request to "/rest/workflows/{wf}/execute" with JSON:
      """
      {}
      """
    And I remember the JSON at "/id" as "exec"
    When I wait for execution "exec" to finish
    And I send a GET request to "/rest/workflows/{wf}/executions"
    Then the response status is 200
    And the JSON at "/0/id" is "{exec}"
