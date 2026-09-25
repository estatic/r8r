@spec-2.4 @spec-6.2 @phase-1
Feature: Per-node settings: disabled, retries, error handling, always output, execute once
  Node flags live on the node object, not in parameters: `disabled`,
  `retryOnFail` + `maxTries` + `waitBetweenTries`, `onError`
  (stopWorkflow | continueRegularOutput | continueErrorOutput),
  `alwaysOutputData` and `executeOnce`.

  Background:
    Given a mock HTTP service

  Scenario: A disabled node passes its input through unchanged
    Given a workflow with nodes:
      | name     | type          | disabled |
      | Start    | manualTrigger |          |
      | Rewrite  | set           | true     |
      | After    | noOp          |          |
    And the node "Rewrite" sets the fields:
      """
      {"rewritten": true}
      """
    And the connections "Start -> Rewrite -> After"
    And the trigger outputs the items:
      """
      [{"original": true}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs:
      """
      [{"original": true}]
      """

  Scenario: A node retries until it succeeds
    Given the mock service responds to GET "/flaky" with status 500 the first 2 times
    And the mock service responds to GET "/flaky" with status 200 and body:
      """
      {"ok": true}
      """
    And a workflow with nodes:
      | name  | type          | retryOnFail | maxTries | waitBetweenTries | parameters                                            |
      | Start | manualTrigger |             |          |                  |                                                       |
      | Call  | httpRequest   | true        | 3        | 100              | {"url": "%{MOCK_URL}/flaky", "options": {}}           |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution succeeds
    And the node "Call" outputs:
      """
      [{"ok": true}]
      """
    And the mock service received 3 requests to "/flaky"

  Scenario: A node gives up after maxTries attempts
    Given the mock service responds to GET "/down" with status 503
    And a workflow with nodes:
      | name  | type          | retryOnFail | maxTries | waitBetweenTries | parameters                                   |
      | Start | manualTrigger |             |          |                  |                                              |
      | Call  | httpRequest   | true        | 2        | 0                | {"url": "%{MOCK_URL}/down", "options": {}}   |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution fails
    And the mock service received 2 requests to "/down"

  Scenario: Retries wait between attempts
    Given the mock service responds to GET "/slow-recover" with status 500 the first 1 time
    And the mock service responds to GET "/slow-recover" with status 200
    And a workflow with nodes:
      | name  | type          | retryOnFail | maxTries | waitBetweenTries | parameters                                         |
      | Start | manualTrigger |             |          |                  |                                                    |
      | Call  | httpRequest   | true        | 2        | 1500             | {"url": "%{MOCK_URL}/slow-recover", "options": {}} |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution succeeds
    And run 0 of the node "Call" took at least 1500 ms

  Scenario: onError "stopWorkflow" (the default) fails the execution
    Given the mock service responds to GET "/missing" with status 404
    And a workflow with nodes:
      | name  | type          | parameters                                     |
      | Start | manualTrigger |                                                |
      | Call  | httpRequest   | {"url": "%{MOCK_URL}/missing", "options": {}}  |
      | After | noOp          |                                                |
    And the connections "Start -> Call -> After"
    When I execute the workflow
    Then the execution fails
    And the node "After" was not executed

  Scenario: onError "continueRegularOutput" passes an error item downstream
    Given the mock service responds to GET "/missing" with status 404
    And a workflow with nodes:
      | name  | type          | onError               | parameters                                     |
      | Start | manualTrigger |                       |                                                |
      | Call  | httpRequest   | continueRegularOutput | {"url": "%{MOCK_URL}/missing", "options": {}}  |
      | After | noOp          |                       |                                                |
    And the connections "Start -> Call -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: onError "continueErrorOutput" routes failed items to the error output
    Given the mock service responds to GET "/missing" with status 404
    And a workflow with nodes:
      | name     | type          | onError             | parameters                                     |
      | Start    | manualTrigger |                     |                                                |
      | Call     | httpRequest   | continueErrorOutput | {"url": "%{MOCK_URL}/missing", "options": {}}  |
      | Success  | noOp          |                     |                                                |
      | Handle   | noOp          |                     |                                                |
    And the connections:
      """
      Start -> Call
      Call:0 -> Success
      Call:1 -> Handle
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Call" is empty
    And output 1 of the node "Call" has 1 item
    And the node "Success" was not executed
    And the node "Handle" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: continueErrorOutput splits good and bad items of one run
    Given a workflow with nodes:
      | name   | type          | onError             |
      | Start  | manualTrigger |                     |
      | Check  | code          | continueErrorOutput |
      | Good   | noOp          |                     |
      | Bad    | noOp          |                     |
    And the node "Check" runs the JavaScript for each item:
      """
      if ($json.n < 0) { throw new Error('negative: ' + $json.n); }
      return $json;
      """
    And the connections:
      """
      Start -> Check
      Check:0 -> Good
      Check:1 -> Bad
      """
    And the trigger outputs the items:
      """
      [{"n": 1}, {"n": -1}, {"n": 2}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Good" outputs:
      """
      [{"n": 1}, {"n": 2}]
      """
    And the node "Bad" outputs items matching:
      """
      [{"error": "$contains:-1"}]
      """

  Scenario: alwaysOutputData emits an empty item when a node outputs nothing
    Given a workflow with nodes:
      | name   | type          | alwaysOutputData | parameters |
      | Start  | manualTrigger |                  |            |
      | None   | filter        | true             | {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2}, "conditions": [{"id": "c1", "leftValue": "={{ $json.x }}", "rightValue": 100, "operator": {"type": "number", "operation": "gt"}}], "combinator": "and"}, "options": {}} |
      | After  | noOp          |                  |            |
    And the connections "Start -> None -> After"
    And the trigger outputs the items:
      """
      [{"x": 1}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs:
      """
      [{}]
      """

  Scenario: executeOnce runs a node for the first item only
    Given the mock service responds to GET "/once" with status 200 and body:
      """
      {"called": true}
      """
    And a workflow with nodes:
      | name  | type          | executeOnce | parameters                                  |
      | Start | manualTrigger |             |                                             |
      | Call  | httpRequest   | true        | {"url": "%{MOCK_URL}/once", "options": {}}  |
    And the connections "Start -> Call"
    And the trigger outputs the items:
      """
      [{"i": 1}, {"i": 2}, {"i": 3}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Call" outputs 1 item
    And the mock service received 1 request to "/once"
