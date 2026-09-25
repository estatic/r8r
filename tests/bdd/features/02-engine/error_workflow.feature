@spec-2.4 @spec-6.2 @phase-2
Feature: Error workflows
  A workflow may name an error workflow in `settings.errorWorkflow`. When a
  production execution fails, the error workflow runs with an Error Trigger
  item describing the failed execution and workflow.

  Background:
    Given a running r8r server with an owner and an API key
    And a workflow named "On error" with nodes:
      | name   | type         |
      | Failed | errorTrigger |
      | Record | set          |
    And the node "Record" sets the fields:
      """
      {
        "workflowName": "={{ $json.workflow.name }}",
        "message": "={{ $json.execution.error.message }}",
        "lastNode": "={{ $json.execution.lastNodeExecuted }}",
        "mode": "={{ $json.execution.mode }}"
      }
      """
    And the connections "Failed -> Record"
    And the workflow "On error" is created
    And a workflow named "Fragile" with nodes:
      | name    | type         | parameters                                                                            |
      | Webhook | webhook      | {"httpMethod": "POST", "path": "fragile", "responseMode": "onReceived", "options": {}} |
      | Explode | stopAndError | {"errorType": "errorMessage", "errorMessage": "Inventory service unavailable"}         |
    And the connections "Webhook -> Explode"
    And the workflow setting "errorWorkflow" is "%{WORKFLOW_ID:On error}"

  Scenario: A failed production execution triggers the error workflow
    Given the workflow "Fragile" is active
    When I send a POST request to "/webhook/fragile" with body:
      """
      {}
      """
    And I wait for the execution of "On error" to finish
    Then the execution succeeds
    And the node "Record" outputs:
      """
      [{"workflowName": "Fragile", "message": "Inventory service unavailable", "lastNode": "Explode", "mode": "webhook"}]
      """

  Scenario: A successful execution does not trigger the error workflow
    Given I edit the workflow "Fragile"
    And the node "Explode" is disabled
    And the workflow "Fragile" is active
    When I send a POST request to "/webhook/fragile" with body:
      """
      {}
      """
    And I wait for the execution of "Fragile" to finish
    Then the execution succeeds
    And the workflow "On error" has 0 executions

  Scenario: A failure handled by continueErrorOutput is not an execution error
    Given I edit the workflow "Fragile"
    And the node "Explode" has the property "onError" set to "continueErrorOutput"
    And the workflow "Fragile" is active
    When I send a POST request to "/webhook/fragile" with body:
      """
      {}
      """
    And I wait for the execution of "Fragile" to finish
    Then the execution succeeds
    And the workflow "On error" has 0 executions
