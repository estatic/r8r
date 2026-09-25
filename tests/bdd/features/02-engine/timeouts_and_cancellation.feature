@spec-6.2 @spec-8.3 @phase-2
Feature: Execution timeouts and cancellation
  Executions can be cancelled cooperatively (checked between nodes and in
  HTTP helpers) and are hard-stopped after `settings.executionTimeout`
  seconds. Every accepted execution ends in success, error, canceled or
  crashed; none stays "running".

  Background:
    Given a running r8r server with an owner and an API key
    And a mock HTTP service

  Scenario: An execution that exceeds its timeout is stopped
    Given the mock service responds to GET "/slow" with status 200 after 10000 ms
    And a workflow named "Too slow" with nodes:
      | name    | type        | parameters                                                                             |
      | Webhook | webhook     | {"httpMethod": "POST", "path": "too-slow", "responseMode": "onReceived", "options": {}} |
      | Call    | httpRequest | {"url": "%{MOCK_URL}/slow", "options": {}}                                             |
      | After   | noOp        |                                                                                        |
    And the connections "Webhook -> Call -> After"
    And the workflow setting "executionTimeout" is 2
    And the workflow is active
    When I send a POST request to "/webhook/too-slow" with body:
      """
      {}
      """
    And I wait for the execution to finish
    Then the execution status is one of "canceled, error"
    And the node "After" was not executed

  Scenario: A running execution can be stopped from the editor
    Given a workflow named "Stoppable" with nodes:
      | name    | type    | parameters                                                                              |
      | Webhook | webhook | {"httpMethod": "POST", "path": "stoppable", "responseMode": "onReceived", "options": {}} |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 30, "unit": "seconds"}                             |
      | After   | noOp    |                                                                                         |
    And the connections "Webhook -> Pause -> After"
    And the workflow is active
    When I send a POST request to "/webhook/stoppable" with body:
      """
      {}
      """
    And I wait for an execution with the status "running"
    And I stop that execution
    And I wait for that execution to finish
    Then the execution status is "canceled"
    And the node "After" was not executed
