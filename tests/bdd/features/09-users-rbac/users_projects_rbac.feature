@spec-6.10 @spec-6.5 @phase-5
Feature: Users, projects and access control
  Roles follow n8n: global owner/admin/member, project admin/editor/viewer,
  with n8n's scope names. Workflows and credentials belong to projects;
  a member sees only what their projects can access.

  Background:
    Given a running r8r server with an owner and an API key
    And a member user "member@example.com"
    And "member@example.com" has an API key

  Scenario: An invited member can log in
    When I am logged in as "member@example.com"
    And I send a GET request to "/rest/login"
    Then the response status is 200
    And the response JSON matches:
      """
      {"data": {"email": "member@example.com", "role": "global:member"}}
      """

  Scenario: A member cannot see the owner's workflows
    Given a workflow named "Owner's secret flow" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I use the API key of "member@example.com"
    And I send a GET request to "/api/v1/workflows/%{WORKFLOW_ID}"
    Then the response status is one of "403, 404"
    When I send a GET request to "/api/v1/workflows"
    Then the response JSON at "data" has no element matching:
      """
      {"name": "Owner's secret flow"}
      """

  Scenario: A member's workflows live in their personal project
    Given I use the API key of "member@example.com"
    And a workflow named "Member flow" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow is created
    When I use the owner's API key
    And I send a GET request to "/api/v1/workflows/%{WORKFLOW_ID}"
    Then the response status is 200

  Scenario: A member cannot list users
    Given I use the API key of "member@example.com"
    When I send a GET request to "/api/v1/users"
    Then the response status is 403

  Scenario: A workflow cannot use a credential its project cannot access
    Given a mock HTTP service
    And the mock service responds to GET "/private" with status 200
    And the credential "Owner key" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Key", "value": "owner-only"}
      """
    And I use the API key of "member@example.com"
    And a workflow named "Borrower" with nodes:
      | name    | type        | parameters                                                                                                              |
      | Webhook | webhook     | {"httpMethod": "GET", "path": "borrow", "responseMode": "lastNode", "options": {}}                                      |
      | Call    | httpRequest | {"url": "%{MOCK_URL}/private", "authentication": "genericCredentialType", "genericAuthType": "httpHeaderAuth", "options": {}} |
    And the node "Call" uses the "httpHeaderAuth" credential "Owner key"
    And the connections "Webhook -> Call"
    When I activate the workflow
    And I send a GET request to "/webhook/borrow"
    Then the mock service received no requests

  # Team projects are a licensed feature in n8n.
  @n8n-licensed
  Scenario: Team projects share workflows with their members
    Given I use the owner's API key
    When I send a POST request to "/api/v1/projects" with body:
      """
      {"name": "Finance"}
      """
    Then the response status is 201
    And I remember the response JSON at "id" as "PROJECT_ID"
    When I send a POST request to "/api/v1/projects/%{PROJECT_ID}/users" with body:
      """
      {"relations": [{"userId": "%{USER_ID:member@example.com}", "role": "project:viewer"}]}
      """
    Then the response status is a success
    When I send a POST request to "/api/v1/workflows" with body:
      """
      {"name": "Finance report", "nodes": [], "connections": {}, "settings": {}}
      """
    Then the response status is 200
    And I remember the response JSON at "id" as "FINANCE_WF"
    When I send a PUT request to "/api/v1/workflows/%{FINANCE_WF}/transfer" with body:
      """
      {"destinationProjectId": "%{PROJECT_ID}"}
      """
    Then the response status is a success
    When I use the API key of "member@example.com"
    And I send a GET request to "/api/v1/workflows/%{FINANCE_WF}"
    Then the response status is 200
    When I send a DELETE request to "/api/v1/workflows/%{FINANCE_WF}"
    Then the response status is 403
