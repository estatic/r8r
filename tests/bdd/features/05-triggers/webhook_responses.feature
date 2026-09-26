@spec-6.3 @phase-2
Feature: Webhook response modes
  `responseMode` decides when and what the caller gets back: `onReceived`
  answers at once, `lastNode` answers with the last node's output, and
  `responseNode` lets a Respond to Webhook node build the response.

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: onReceived answers immediately with the default message
    Given a workflow named "Fast ack" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "ack", "responseMode": "onReceived", "options": {}}  |
      | Pause   | wait    | {"resume": "timeInterval", "amount": 3, "unit": "seconds"}                         |
    And the connections "Webhook -> Pause"
    And the workflow is active
    When I send a POST request to "/webhook/ack" with body:
      """
      {}
      """
    Then the response status is 200
    And the response took at most 1500 ms
    And the response JSON is:
      """
      {"message": "Workflow was started"}
      """

  Scenario: onReceived with a custom status code and no body
    Given a workflow named "Accepted" with nodes:
      | name    | type    | parameters                                                                                                    |
      | Webhook | webhook | {"httpMethod": "POST", "path": "accepted", "responseMode": "onReceived", "options": {"responseCode": {"values": {"responseCode": 202}}, "noResponseBody": true}} |
    And the workflow is active
    When I send a POST request to "/webhook/accepted" with body:
      """
      {}
      """
    Then the response status is 202
    And the response body is ""

  Scenario: lastNode returns all entries when asked
    Given a workflow named "All entries" with nodes:
      | name    | type    | parameters                                                                                                   |
      | Webhook | webhook | {"httpMethod": "GET", "path": "all", "responseMode": "lastNode", "responseData": "allEntries", "options": {}} |
      | Make    | code    |                                                                                                              |
    And the node "Make" runs the JavaScript:
      """
      return [{ json: { n: 1 } }, { json: { n: 2 } }];
      """
    And the connections "Webhook -> Make"
    And the workflow is active
    When I send a GET request to "/webhook/all"
    Then the response status is 200
    And the response JSON is:
      """
      [{"n": 1}, {"n": 2}]
      """

  Scenario: lastNode answers 500 when the execution fails
    Given a workflow named "Broken" with nodes:
      | name    | type         | parameters                                                                         |
      | Webhook | webhook      | {"httpMethod": "GET", "path": "broken", "responseMode": "lastNode", "options": {}} |
      | Fail    | stopAndError | {"errorType": "errorMessage", "errorMessage": "nope"}                              |
    And the connections "Webhook -> Fail"
    And the workflow is active
    When I send a GET request to "/webhook/broken"
    Then the response status is 500
    And the response JSON matches:
      """
      {"message": "Error in workflow"}
      """

  Scenario: Respond to Webhook sets the status, headers and JSON body
    Given a workflow named "Custom response" with nodes:
      | name    | type             | parameters                                                                            |
      | Webhook | webhook          | {"httpMethod": "POST", "path": "custom", "responseMode": "responseNode", "options": {}} |
      | Respond | respondToWebhook |                                                                                       |
      | After   | noOp             |                                                                                       |
    And the node "Respond" has parameters:
      """
      {"respondWith": "json", "responseBody": "={{ { \"id\": $json.body.id, \"status\": \"created\" } }}",
       "options": {"responseCode": 201, "responseHeaders": {"entries": [{"name": "X-Order-Id", "value": "={{ $json.body.id }}"}]}}}
      """
    And the connections "Webhook -> Respond -> After"
    And the workflow is active
    When I send a POST request to "/webhook/custom" with body:
      """
      {"id": "ord-9"}
      """
    Then the response status is 201
    And the response header "x-order-id" is "ord-9"
    And the response JSON is:
      """
      {"id": "ord-9", "status": "created"}
      """
    And I wait for the execution to finish
    And the node "After" was executed 1 time

  Scenario: Respond to Webhook with text
    Given a workflow named "Text response" with nodes:
      | name    | type             | parameters                                                                                 |
      | Webhook | webhook          | {"httpMethod": "GET", "path": "text", "responseMode": "responseNode", "options": {}}       |
      | Respond | respondToWebhook | {"respondWith": "text", "responseBody": "=Hello {{ $json.query.name }}", "options": {}}    |
    And the connections "Webhook -> Respond"
    And the workflow is active
    When I send a GET request to "/webhook/text?name=Ada"
    Then the response status is 200
    And the response body is "Hello Ada"

  Scenario: Respond to Webhook with a redirect
    Given a workflow named "Redirect" with nodes:
      | name    | type             | parameters                                                                                       |
      | Webhook | webhook          | {"httpMethod": "GET", "path": "go", "responseMode": "responseNode", "options": {}}               |
      | Respond | respondToWebhook | {"respondWith": "redirect", "redirectURL": "https://example.com/landing", "options": {}}          |
    And the connections "Webhook -> Respond"
    And the workflow is active
    When I send a GET request to "/webhook/go"
    Then the response status is 307
    And the response header "location" is "https://example.com/landing"

  Scenario: A responseNode workflow that never responds still answers the caller
    Given a workflow named "Forgot to respond" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "GET", "path": "forgot", "responseMode": "responseNode", "options": {}} |
      | Noop    | noOp    |                                                                                        |
    And the connections "Webhook -> Noop"
    And the workflow is active
    When I send a GET request to "/webhook/forgot"
    Then the response status is one of "200, 500"
    And the response took at most 10000 ms
