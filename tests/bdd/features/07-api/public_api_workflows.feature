@spec-6.9 @spec-4.1 @phase-2
Feature: Public API: workflows
  `/api/v1` is n8n's public API, authenticated with `X-N8N-API-KEY` and
  scoped per key. Existing API clients must work unchanged (goal G2).

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: Requests without an API key are rejected
    Given I am not authenticated
    When I send a GET request to "/api/v1/workflows"
    Then the response status is 401

  Scenario: Requests with an unknown API key are rejected
    Given I use the API key "n8n_api_this-is-not-a-real-key"
    When I send a GET request to "/api/v1/workflows"
    Then the response status is 401

  Scenario: Create a workflow and read it back unchanged
    When I send a POST request to "/api/v1/workflows" with body:
      """
      {
        "name": "Created via API",
        "nodes": [
          {"id": "n1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
          {"id": "n2", "name": "Set", "type": "n8n-nodes-base.set", "typeVersion": 3.4, "position": [200, 0],
           "parameters": {"mode": "manual", "assignments": {"assignments": [{"id": "a", "name": "x", "value": "1", "type": "string"}]}, "options": {}}}
        ],
        "connections": {"Start": {"main": [[{"node": "Set", "type": "main", "index": 0}]]}},
        "settings": {"executionOrder": "v1"}
      }
      """
    Then the response status is 200
    And the response JSON matches:
      """
      {"id": "$string", "name": "Created via API", "active": false, "createdAt": "$datetime", "updatedAt": "$datetime", "versionId": "$string"}
      """
    And I remember the response JSON at "id" as "NEW_ID"
    When I send a GET request to "/api/v1/workflows/%{NEW_ID}"
    Then the response status is 200
    And the response JSON matches:
      """
      {
        "name": "Created via API",
        "nodes": [
          {"id": "n1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
          {"id": "n2", "name": "Set", "typeVersion": 3.4,
           "parameters": {"mode": "manual", "assignments": {"assignments": [{"id": "a", "name": "x", "value": "1", "type": "string"}]}, "options": {}}}
        ],
        "connections": {"Start": {"main": [[{"node": "Set", "type": "main", "index": 0}]]}},
        "settings": {"executionOrder": "v1"}
      }
      """

  Scenario: Creating a workflow without nodes is rejected
    When I send a POST request to "/api/v1/workflows" with body:
      """
      {"name": "Incomplete"}
      """
    Then the response status is 400

  Scenario: Read-only properties cannot be set on create
    When I send a POST request to "/api/v1/workflows" with body:
      """
      {"name": "Sneaky", "nodes": [], "connections": {}, "settings": {}, "active": true}
      """
    Then the response status is 400

  Scenario: Update a workflow
    Given a workflow named "To update" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I send a PUT request to "/api/v1/workflows/%{WORKFLOW_ID}" with body:
      """
      {"name": "Updated name", "nodes": [{"id": "n1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}}], "connections": {}, "settings": {"timezone": "Europe/Berlin"}}
      """
    Then the response status is 200
    And the response JSON matches:
      """
      {"name": "Updated name", "settings": {"timezone": "Europe/Berlin"}}
      """

  Scenario: Delete a workflow
    Given a workflow named "To delete" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I send a DELETE request to "/api/v1/workflows/%{WORKFLOW_ID}"
    Then the response status is 200
    When I send a GET request to "/api/v1/workflows/%{WORKFLOW_ID}"
    Then the response status is 404

  Scenario: Activate and deactivate
    Given a workflow named "Toggle" with nodes:
      | name    | type    | parameters                                                                        |
      | Webhook | webhook | {"httpMethod": "GET", "path": "toggle", "responseMode": "onReceived", "options": {}} |
    And the workflow is created
    When I send a POST request to "/api/v1/workflows/%{WORKFLOW_ID}/activate"
    Then the response status is 200
    And the response JSON at "active" is true
    When I send a POST request to "/api/v1/workflows/%{WORKFLOW_ID}/deactivate"
    Then the response status is 200
    And the response JSON at "active" is false

  Scenario: A workflow without a trigger cannot be activated
    Given a workflow named "No trigger" with nodes:
      | name | type |
      | Noop | noOp |
    And the workflow is created
    When I send a POST request to "/api/v1/workflows/%{WORKFLOW_ID}/activate"
    Then the response status is 400

  Scenario: Listing is paginated with a cursor
    Given a workflow named "First" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    And a workflow named "Second" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I send a GET request to "/api/v1/workflows?limit=1"
    Then the response status is 200
    And the response JSON at "data" has 1 element
    And the response JSON at "nextCursor" matches:
      """
      "$nonempty"
      """
    And I remember the response JSON at "nextCursor" as "CURSOR"
    When I send a GET request to "/api/v1/workflows?limit=1&cursor=%{CURSOR}"
    Then the response JSON at "data" has 1 element
    And the response JSON at "nextCursor" is null

  Scenario: Listing can filter by active state and name
    Given a workflow named "Live hook" with nodes:
      | name    | type    | parameters                                                                     |
      | Webhook | webhook | {"httpMethod": "GET", "path": "live", "responseMode": "onReceived", "options": {}} |
    And the workflow is active
    And a workflow named "Draft" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I send a GET request to "/api/v1/workflows?active=true"
    Then the response JSON at "data" has 1 element
    And the response JSON at "data[0].name" is "Live hook"
    When I send a GET request to "/api/v1/workflows?name=Draft"
    Then the response JSON at "data" has 1 element

  Scenario: Tags can be created and attached to workflows
    Given a workflow named "Tagged" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I send a POST request to "/api/v1/tags" with body:
      """
      {"name": "billing"}
      """
    Then the response status is 201
    And I remember the response JSON at "id" as "TAG_ID"
    When I send a POST request to "/api/v1/tags" with body:
      """
      {"name": "billing"}
      """
    Then the response status is 409
    When I send a PUT request to "/api/v1/workflows/%{WORKFLOW_ID}/tags" with body:
      """
      [{"id": "%{TAG_ID}"}]
      """
    Then the response status is 200
    And the response JSON at "" contains an element matching:
      """
      {"name": "billing"}
      """
    When I send a GET request to "/api/v1/workflows?tags=billing"
    Then the response JSON at "data" has 1 element

  Scenario: A key without the workflow:create scope cannot create workflows
    Given "owner" has an API key with the scopes "workflow:read, workflow:list"
    And I use the API key of "owner"
    When I send a POST request to "/api/v1/workflows" with body:
      """
      {"name": "Forbidden", "nodes": [], "connections": {}, "settings": {}}
      """
    Then the response status is 403
    When I send a GET request to "/api/v1/workflows"
    Then the response status is 200
