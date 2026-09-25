@spec-6.2 @phase-2
Feature: Sub-workflows
  The Execute Workflow node runs another workflow as a child execution,
  waiting for its result or firing and forgetting. The child's
  `settings.callerPolicy` decides who may call it.

  Background:
    Given a running r8r server with an owner and an API key
    And a workflow named "Child" with nodes:
      | name    | type                   | parameters                     |
      | Input   | executeWorkflowTrigger | {"inputSource": "passthrough"} |
      | Compute | set                    |                                |
    And the node "Compute" sets the fields:
      """
      {"total": "={{ $json.price * $json.qty }}"}
      """
    And the connections "Input -> Compute"
    And the workflow setting "callerPolicy" is "any"
    # n8n 2.x refuses to publish a parent whose sub-workflow is unpublished.
    And the workflow "Child" is active
    And a workflow named "Parent" with nodes:
      | name    | type            | parameters                                                                            |
      | Webhook | webhook         | {"httpMethod": "POST", "path": "parent", "responseMode": "lastNode", "options": {}}   |
      | Prepare | set             |                                                                                       |
      | Call    | executeWorkflow |                                                                                       |
    And the node "Prepare" sets the fields:
      """
      {"price": "={{ $json.body.price }}", "qty": "={{ $json.body.qty }}"}
      """
    And the node "Call" has parameters:
      """
      {"source": "database", "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Child}", "mode": "id"}, "options": {"waitForSubWorkflow": true}}
      """
    And the connections "Webhook -> Prepare -> Call"

  Scenario: The parent receives the child's output
    Given the workflow "Parent" is active
    When I send a POST request to "/webhook/parent" with body:
      """
      {"price": 2.5, "qty": 4}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"total": 10}
      """
    And the workflow "Child" has 1 execution

  Scenario: Fire-and-forget returns the parent's own items immediately
    Given I edit the workflow "Parent"
    And the node "Call" has parameters:
      """
      {"source": "database", "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Child}", "mode": "id"}, "options": {"waitForSubWorkflow": false}}
      """
    And the workflow "Parent" is active
    When I send a POST request to "/webhook/parent" with body:
      """
      {"price": 1, "qty": 3}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"price": 1, "qty": 3}
      """
    And I wait for the execution of "Child" to finish
    And the node "Compute" outputs:
      """
      [{"total": 3}]
      """

  Scenario: A child that allows no callers refuses the call
    Given a workflow named "Locked child" with nodes:
      | name  | type                   | parameters                     |
      | Input | executeWorkflowTrigger | {"inputSource": "passthrough"} |
    And the workflow setting "callerPolicy" is "none"
    And the workflow "Locked child" is active
    And I edit the workflow "Parent"
    And the node "Call" has parameters:
      """
      {"source": "database", "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Locked child}", "mode": "id"}, "options": {"waitForSubWorkflow": true}}
      """
    And the workflow "Parent" is active
    When I send a POST request to "/webhook/parent" with body:
      """
      {"price": 1, "qty": 1}
      """
    Then the response status is 500
    And I wait for the execution of "Parent" to finish
    And the execution fails
    And the node "Call" failed with an error containing "not allowed"

  Scenario: The child execution records its parent
    Given the workflow "Parent" is active
    When I send a POST request to "/webhook/parent" with body:
      """
      {"price": 1, "qty": 1}
      """
    And I wait for the execution of "Child" to finish
    Then the execution succeeds
    And the execution mode is "integrated"
