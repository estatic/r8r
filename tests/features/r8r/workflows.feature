@r8r
Feature: Workflow management
  Users create, read, update and delete workflows through the r8r REST API.
  Node execution settings are validated when a workflow is saved.

  Background:
    Given I am logged in as "wf@example.com"

  Scenario: Create a workflow and read it back
    When I send a POST request to "/rest/workflows" with JSON:
      """
      {"name": "hello", "nodes": [{"id": "t", "node_type": "core.manualTrigger", "position": [0, 0], "parameters": {}}], "connections": []}
      """
    Then the response status is 201
    And I remember the JSON at "/id" as "wf"
    When I send a GET request to "/rest/workflows/{wf}"
    Then the response status is 200
    And the JSON at "/name" is "hello"
    And the JSON at "/active" is false
    And the JSON at "/nodes" has 1 item

  Scenario: Created workflows are listed
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "listed", "nodes": [], "connections": []}
      """
    When I send a GET request to "/rest/workflows"
    Then the response status is 200
    And the JSON at "/0/name" is "listed"

  Scenario: Update renames a workflow
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "before", "nodes": [], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a PUT request to "/rest/workflows/{wf}" with JSON:
      """
      {"name": "after", "nodes": [], "connections": []}
      """
    Then the response status is 200
    And the JSON at "/name" is "after"

  Scenario: Delete a workflow
    Given I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "doomed", "nodes": [], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a DELETE request to "/rest/workflows/{wf}"
    Then the response status is 204
    When I send a GET request to "/rest/workflows/{wf}"
    Then the response status is 404

  Scenario: Reading a workflow that does not exist
    When I send a GET request to "/rest/workflows/00000000-0000-0000-0000-000000000000"
    Then the response status is 404

  Scenario: Anonymous requests are rejected
    Given I am not logged in
    When I send a GET request to "/rest/workflows"
    Then the response status is 401

  Scenario: A forged session token is rejected
    Given my session token is "not-a-real-jwt"
    When I send a GET request to "/rest/workflows"
    Then the response status is 401

  Scenario Outline: Out-of-range node settings are rejected on save
    When I send a POST request to "/rest/workflows" with JSON:
      """
      {"name": "bad", "nodes": [{"id": "n1", "node_type": "core.set", "position": [0, 0], "parameters": {}, "settings": <settings>}], "connections": []}
      """
    Then the response status is 400
    And the response body contains "<message>"

    Examples:
      | settings                                   | message                                          |
      | {"retry": {"max_tries": 1, "wait_ms": 0}}  | node n1: retry.max_tries must be between 2 and 10 |
      | {"retry": {"max_tries": 11, "wait_ms": 0}} | node n1: retry.max_tries must be between 2 and 10 |
      | {"retry": {"max_tries": 3, "wait_ms": 60001}} | node n1: retry.wait_ms must be between 0 and 60000 |
      | {"timeout_ms": 0}                          | node n1: timeout_ms must be between 1 and 3600000 |

  Scenario: In-range node settings are stored
    When I send a POST request to "/rest/workflows" with JSON:
      """
      {"name": "ok", "nodes": [{"id": "n1", "node_type": "core.set", "position": [0, 0], "parameters": {}, "settings": {"retry": {"max_tries": 3, "wait_ms": 500}, "timeout_ms": 1000, "continue_on_fail": true}}], "connections": []}
      """
    Then the response status is 201
    And the JSON at "/nodes/0/settings" equals:
      """
      {"retry": {"max_tries": 3, "wait_ms": 500}, "timeout_ms": 1000, "continue_on_fail": true}
      """
