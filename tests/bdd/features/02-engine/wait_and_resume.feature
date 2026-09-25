@spec-2.4 @spec-6.2 @phase-2
Feature: Wait node and resuming executions
  A Wait node pauses the execution. Long waits persist the full execution
  state with status `waiting` and `waitTill`; any process can resume it
  later, on time or when its resume URL is called. In n8n 2.x the resume
  URL (`$execution.resumeUrl`, `/webhook-waiting/:executionId`) carries a
  signature, and an unsigned call is refused.

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: A short timed wait delays the rest of the execution
    Given a workflow named "Short wait" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "POST", "path": "short-wait", "responseMode": "lastNode", "options": {}} |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 2, "unit": "seconds"}                             |
      | Done    | set     |                                                                                        |
    And the node "Done" sets the fields:
      """
      {"done": true}
      """
    And the connections "Webhook -> Pause -> Done"
    And the workflow is active
    When I send a POST request to "/webhook/short-wait" with body:
      """
      {}
      """
    Then the response status is 200
    And the response took at least 2000 ms
    And the response JSON is:
      """
      {"done": true}
      """

  Scenario: Waiting for a webhook persists the execution as waiting, then resumes on call
    Given a workflow named "Approval" with nodes:
      | name     | type    | parameters                                                                            |
      | Webhook  | webhook | {"httpMethod": "POST", "path": "approval", "responseMode": "onReceived", "options": {}} |
      | Resume   | set     |                                                                                       |
      | Approval | wait    | {"resume": "webhook", "httpMethod": "POST", "options": {}}                            |
      | Decision | set     |                                                                                       |
    And the node "Resume" sets the fields:
      """
      {"resumeUrl": "={{ $execution.resumeUrl }}", "request": "={{ $json.body.request }}"}
      """
    And the node "Decision" sets the fields:
      """
      {"approved": "={{ $json.body.approved }}", "request": "={{ $('Resume').item.json.request }}"}
      """
    And the connections "Webhook -> Resume -> Approval -> Decision"
    And the workflow is active
    When I send a POST request to "/webhook/approval" with body:
      """
      {"request": "laptop"}
      """
    Then the response status is 200
    When I wait for an execution with the status "waiting"
    Then the execution status is "waiting"
    And the execution has a wait time in the future
    When I send a POST request to "/webhook-waiting/%{EXECUTION_ID}" with body:
      """
      {"approved": true}
      """
    Then the response status is a client error
    When I remember the field "resumeUrl" of item 0 from the node "Resume" as "RESUME_URL"
    And I send a POST request to "%{RESUME_URL}" with body:
      """
      {"approved": true}
      """
    Then the response status is 200
    When I wait for that execution to finish
    Then the execution succeeds
    And the node "Decision" outputs:
      """
      [{"approved": true, "request": "laptop"}]
      """
    And the node "Resume" outputs items matching:
      """
      [{"resumeUrl": "$regex:/webhook-waiting/%{EXECUTION_ID}$"}]
      """

  Scenario: Resuming a finished execution is refused
    Given a workflow named "Once" with nodes:
      | name    | type    | parameters                                                                        |
      | Webhook | webhook | {"httpMethod": "POST", "path": "once", "responseMode": "onReceived", "options": {}} |
      | Resume  | set     |                                                                                   |
      | Hold    | wait    | {"resume": "webhook", "httpMethod": "POST", "options": {}}                        |
    And the node "Resume" sets the fields:
      """
      {"resumeUrl": "={{ $execution.resumeUrl }}"}
      """
    And the connections "Webhook -> Resume -> Hold"
    And the workflow is active
    When I send a POST request to "/webhook/once" with body:
      """
      {}
      """
    And I wait for an execution with the status "waiting"
    And I remember the field "resumeUrl" of item 0 from the node "Resume" as "RESUME_URL"
    And I send a POST request to "%{RESUME_URL}" with body:
      """
      {}
      """
    And I wait for that execution to finish
    And I send a POST request to "%{RESUME_URL}" with body:
      """
      {}
      """
    Then the response status is one of "404, 409"

  Scenario: A waiting execution survives a restart and can still be resumed
    Given a workflow named "Durable" with nodes:
      | name    | type    | parameters                                                                           |
      | Webhook | webhook | {"httpMethod": "POST", "path": "durable", "responseMode": "onReceived", "options": {}} |
      | Resume  | set     |                                                                                      |
      | Hold    | wait    | {"resume": "webhook", "httpMethod": "POST", "options": {}}                           |
      | After   | set     |                                                                                      |
    And the node "Resume" sets the fields:
      """
      {"resumeUrl": "={{ $execution.resumeUrl }}"}
      """
    And the node "After" sets the fields:
      """
      {"resumed": true}
      """
    And the connections "Webhook -> Resume -> Hold -> After"
    And the workflow is active
    When I send a POST request to "/webhook/durable" with body:
      """
      {}
      """
    And I wait for an execution with the status "waiting"
    And I remember the field "resumeUrl" of item 0 from the node "Resume" as "RESUME_URL"
    And I restart the r8r server
    And I send a POST request to "%{RESUME_URL}" with body:
      """
      {}
      """
    And I wait for that execution to finish
    Then the execution succeeds
    And the node "After" outputs:
      """
      [{"resumed": true}]
      """

  Scenario: Waiting until a date far in the future marks the execution as waiting
    Given a workflow named "Later" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "later", "responseMode": "onReceived", "options": {}} |
      | Hold    | wait    | {"resume": "specificTime", "dateTime": "2099-01-01T00:00:00"}                        |
    And the connections "Webhook -> Hold"
    And the workflow is active
    When I send a POST request to "/webhook/later" with body:
      """
      {}
      """
    And I wait for an execution with the status "waiting"
    Then the execution has a wait time in the future
