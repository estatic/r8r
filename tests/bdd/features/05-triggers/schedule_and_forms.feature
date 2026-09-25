@spec-6.3 @phase-2
Feature: Schedule and form triggers
  Schedule triggers run on intervals or cron expressions in the workflow's
  timezone, on the leader only. Form triggers serve n8n's form at
  `/form/:webhookId` and start an execution per submission.

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: An interval schedule fires repeatedly while active
    Given a workflow named "Every second" with nodes:
      | name     | type            | parameters                                                    |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
      | Stamp    | set             |                                                               |
    And the node "Stamp" sets the fields:
      """
      {"at": "={{ $json.timestamp }}"}
      """
    And the connections "Schedule -> Stamp"
    And the workflow is active
    Then within 6 seconds the workflow has at least 3 executions
    And every execution of the workflow has the mode "trigger"

  Scenario: The schedule trigger item describes the firing time
    Given a workflow named "Describe" with nodes:
      | name     | type            | parameters                                                     |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow setting "timezone" is "Europe/Paris"
    And the workflow is active
    Then within 5 seconds the workflow has at least 1 executions
    And the node "Schedule" outputs items matching:
      """
      [{"timestamp": "$regex:\\+0[12]:00$", "Timezone": "$contains:Europe/Paris"}]
      """

  Scenario: Deactivating stops the schedule
    Given a workflow named "Stoppable schedule" with nodes:
      | name     | type            | parameters                                                     |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    And within 4 seconds the workflow has at least 1 executions
    When I deactivate the workflow
    And I remember the executions count of the workflow
    Then after 3 seconds the workflow has no new executions

  Scenario: A cron expression with an invalid format is rejected on activation
    Given a workflow named "Bad cron" with nodes:
      | name     | type            | parameters                                                                         |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "cronExpression", "expression": "not a cron"}]}}  |
    When I activate the workflow
    Then the response status is a client error

  Scenario: An active schedule keeps firing after a restart
    Given a workflow named "Durable schedule" with nodes:
      | name     | type            | parameters                                                     |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    When I restart the r8r server
    And I remember the executions count of the workflow
    Then within 5 seconds the workflow has new executions

  Scenario: A form trigger serves an HTML form and runs on submission
    Given a workflow named "Contact" with nodes:
      | name | type        | parameters |
      | Form | formTrigger |            |
      | Save | set         |            |
    And the node "Form" has parameters:
      """
      {"formTitle": "Contact us", "formFields": {"values": [{"fieldLabel": "Name", "requiredField": true}, {"fieldLabel": "Message"}]}, "options": {}}
      """
    And the node "Save" sets the fields:
      """
      {"name": "={{ $json.Name }}", "message": "={{ $json.Message }}"}
      """
    And the connections "Form -> Save"
    And the workflow is active
    When I send a GET request to "/form/%{WEBHOOK_ID:Form}"
    Then the response status is 200
    And the response header "content-type" contains "text/html"
    And the response body contains "Contact us"
    When I submit the form at "/form/%{WEBHOOK_ID:Form}" with the fields:
      | field-0 | Ada   |
      | field-1 | Hello |
    Then the response status is 200
    And I wait for the execution to finish
    And the node "Save" outputs:
      """
      [{"name": "Ada", "message": "Hello"}]
      """
