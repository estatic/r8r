@r8r
Feature: AI Agent configuration
  An AI Agent needs a model and a user message; the provider comes from the
  node or, when absent, from its credential's type. Misconfiguration fails
  the run with every missing setting named at once.

  Background:
    Given I am logged in as "agent@example.com"

  Scenario: Every missing setting is named at once
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "bare-agent", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "agent", "node_type": "ai.agent", "position": [0, 100], "parameters": {}}
      ], "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "agent", "to_input": 0}]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Error"
    And the response body contains "missing required parameters: provider"
    And the response body contains "model, user_message"

  Scenario: The provider is inferred from an OpenAI-compatible credential
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Ollama", "credential_type": "openaiApi", "data": {"api_key": "x", "base_url": "http://127.0.0.1:9/v1"}}
      """
    And I remember the JSON at "/id" as "cred"
    And I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "inferred", "nodes": [
        {"id": "trigger", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}},
        {"id": "agent", "node_type": "ai.agent", "position": [0, 100], "parameters": {"auth": {"credential_id": "{cred}"}}}
      ], "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "agent", "to_input": 0}]}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    And I remember the JSON at "/id" as "exec"
    And I wait for execution "exec" to finish
    Then the JSON at "/status" is "Error"
    And the response body contains "missing required parameters: model, user_message"
    And the response body does not contain "provider ("

  Scenario: An agent referencing a missing tool is refused before running
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "ghost-tool", "nodes": [
        {"id": "agent", "node_type": "ai.agent", "position": [0, 0], "parameters": {"tool_ids": ["00000000-0000-0000-0000-000000000003"]}}
      ], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a POST request to "/rest/workflows/{wf}/execute"
    Then the response status is 400
    And the response body contains "referenced tool 00000000-0000-0000-0000-000000000003 does not exist"
