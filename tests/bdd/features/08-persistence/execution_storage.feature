@spec-7.2 @spec-8.3 @phase-2
Feature: Saving, pruning and recovering executions
  Per-workflow save policies decide which executions are stored. No
  accepted production execution is silently lost: each ends as success,
  error, canceled or crashed, never stuck in "running" after a crash.

  Background:
    Given a running r8r server with an owner and an API key
    And a workflow named "Saved?" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "saved", "responseMode": "lastNode", "options": {}}  |
      | Check   | code    |                                                                                     |
    And the node "Check" runs the JavaScript:
      """
      if ($json.body.fail) { throw new Error('asked to fail'); }
      return [{ json: { ok: true } }];
      """
    And the connections "Webhook -> Check"

  Scenario: Successful executions are not saved when saveDataSuccessExecution is "none"
    Given the workflow setting "saveDataSuccessExecution" is "none"
    And the workflow is active
    When I send a POST request to "/webhook/saved" with body:
      """
      {"fail": false}
      """
    Then the response status is 200
    And the workflow has 0 executions

  Scenario: Failed executions are saved by default even when successes are not
    Given the workflow setting "saveDataSuccessExecution" is "none"
    And the workflow is active
    When I send a POST request to "/webhook/saved" with body:
      """
      {"fail": true}
      """
    Then the workflow has 1 execution

  Scenario: Failed executions are not saved when saveDataErrorExecution is "none"
    Given the workflow setting "saveDataErrorExecution" is "none"
    And the workflow is active
    When I send a POST request to "/webhook/saved" with body:
      """
      {"fail": true}
      """
    Then the workflow has 0 executions

  Scenario: The instance default applies when the workflow has no policy
    Given the environment variable "EXECUTIONS_DATA_SAVE_ON_SUCCESS" is "none"
    And I restart the r8r server
    And the workflow is active
    When I send a POST request to "/webhook/saved" with body:
      """
      {"fail": false}
      """
    Then the workflow has 0 executions

  Scenario: Manual executions are not saved when saveManualExecutions is false
    Given a workflow named "Manual only" with nodes:
      | name  | type          |
      | Start | manualTrigger |
    And the workflow setting "saveManualExecutions" is false
    When I run the workflow manually from the editor
    Then the workflow has 0 executions

  Scenario: Executions are stored with n8n's metadata columns
    Given the workflow is active
    When I send a POST request to "/webhook/saved" with body:
      """
      {"fail": false}
      """
    And I wait for the execution to finish
    Then the execution result matches:
      """
      {"finished": true, "mode": "webhook", "status": "success", "retryOf": null, "waitTill": null, "workflowId": "%{WORKFLOW_ID}"}
      """

  Scenario: An execution interrupted by a crash is marked crashed on the next start
    Given a workflow named "Long runner" with nodes:
      | name    | type    | parameters                                                                            |
      | Webhook | webhook | {"httpMethod": "POST", "path": "long", "responseMode": "onReceived", "options": {}}   |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 20, "unit": "seconds"}                           |
    And the connections "Webhook -> Pause"
    And the workflow is active
    When I send a POST request to "/webhook/long" with body:
      """
      {}
      """
    And I wait for an execution with the status "running"
    And the r8r server is killed
    And I start the r8r server again
    And I wait for that execution to finish
    Then the execution status is "crashed"

  Scenario: Graceful shutdown lets in-flight executions finish
    Given the environment variable "N8N_GRACEFUL_SHUTDOWN_TIMEOUT" is "30"
    And I restart the r8r server
    And a workflow named "Almost done" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "POST", "path": "finish", "responseMode": "onReceived", "options": {}}  |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 3, "unit": "seconds"}                             |
      | Done    | noOp    |                                                                                        |
    And the connections "Webhook -> Pause -> Done"
    And the workflow is active
    When I send a POST request to "/webhook/finish" with body:
      """
      {}
      """
    And I wait for an execution with the status "running"
    And I stop the r8r server gracefully
    Then the server exited cleanly within 30 seconds
    When I start the r8r server again
    And I wait for that execution to finish
    Then the execution succeeds
    And the node "Done" was executed 1 time

  Scenario: Data survives a restart
    Given the workflow is active
    And I send a POST request to "/webhook/saved" with body:
      """
      {"fail": false}
      """
    And I wait for the execution to finish
    When I restart the r8r server
    Then the workflow has 1 execution
    When I send a GET request to "/api/v1/workflows/%{WORKFLOW_ID}"
    Then the response status is 200
    And the response JSON at "active" is true
