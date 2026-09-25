@spec-2.4 @spec-6.2 @spec-3.1
Feature: Pin data and partial execution
  Pinned output replaces a node's execution in manual runs, which makes the
  editor's build-test loop fast. Partial execution runs "up to node X" and
  can reuse earlier run data instead of re-running upstream nodes.

  @phase-2
  Scenario: In a manual run a pinned node is not executed; its pinned items flow downstream
    Given a running r8r server with an owner and an API key
    And a mock HTTP service
    And a workflow named "Pinned fetch" with nodes:
      | name    | type          | parameters                                      |
      | Start   | manualTrigger |                                                 |
      | Fetch   | httpRequest   | {"url": "%{MOCK_URL}/customers", "options": {}} |
      | Shape   | set           |                                                 |
    And the node "Shape" sets the fields:
      """
      {"customer": "={{ $json.name }}"}
      """
    And the connections "Start -> Fetch -> Shape"
    And the node "Fetch" is pinned with the items:
      """
      [{"name": "Ada"}, {"name": "Grace"}]
      """
    When I run the workflow manually from the editor
    And I wait for that execution to finish
    Then the execution succeeds
    And the node "Shape" outputs:
      """
      [{"customer": "Ada"}, {"customer": "Grace"}]
      """
    And the mock service received no requests

  @phase-2
  Scenario: In a manual run pinned data on the trigger is the input of the run
    Given a running r8r server with an owner and an API key
    And a workflow named "Pinned trigger" with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Echo  | noOp          |
    And the connections "Start -> Echo"
    And the node "Start" is pinned with the items:
      """
      [{"pinned": 1}, {"pinned": 2}]
      """
    When I run the workflow manually from the editor
    And I wait for that execution to finish
    Then the node "Echo" outputs:
      """
      [{"pinned": 1}, {"pinned": 2}]
      """

  @phase-1
  Scenario: Headless (cli mode) executions ignore pin data, as in n8n
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Echo  | noOp          |
    And the connections "Start -> Echo"
    And the node "Echo" is pinned with the items:
      """
      [{"pinned": true}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Echo" outputs:
      """
      [{}]
      """

  @phase-2
  Scenario: Production executions ignore pin data
    Given a running r8r server with an owner and an API key
    And a workflow named "Pinned webhook" with nodes:
      | name    | type    | parameters                                                                      |
      | Webhook | webhook | {"httpMethod": "POST", "path": "pinned", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                 |
    And the node "Echo" sets the fields:
      """
      {"got": "={{ $json.body.value }}"}
      """
    And the connections "Webhook -> Echo"
    And the node "Webhook" is pinned with the items:
      """
      [{"body": {"value": "from pin data"}}]
      """
    And the workflow is saved from the editor
    And the workflow is active
    When I send a POST request to "/webhook/pinned" with body:
      """
      {"value": "from the request"}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"got": "from the request"}
      """

  @phase-2
  Scenario: Running up to a destination node stops after that node
    Given a running r8r server with an owner and an API key
    And a workflow named "Partial" with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | A     | noOp          |
      | B     | noOp          |
      | C     | noOp          |
    And the connections "Start -> A -> B -> C"
    When I run the workflow manually from the editor up to the node "B"
    And I wait for that execution to finish
    Then the execution succeeds
    And the node "B" was executed 1 time
    And the node "C" was not executed

  @phase-2
  Scenario: A partial re-run reuses upstream run data instead of re-executing it
    Given a running r8r server with an owner and an API key
    And a mock HTTP service
    And the mock service responds to GET "/expensive" with status 200 and body:
      """
      {"value": 7}
      """
    And a workflow named "Rerun" with nodes:
      | name      | type          | parameters                                      |
      | Start     | manualTrigger |                                                 |
      | Expensive | httpRequest   | {"url": "%{MOCK_URL}/expensive", "options": {}} |
      | Double    | set           |                                                 |
    And the node "Double" sets the fields:
      """
      {"doubled": "={{ $json.value * 2 }}"}
      """
    And the connections "Start -> Expensive -> Double"
    When I run the workflow manually from the editor
    And I wait for that execution to finish
    And I re-run the workflow manually from the node "Double" reusing the previous run data
    And I wait for that execution to finish
    Then the execution succeeds
    And the node "Double" outputs:
      """
      [{"doubled": 14}]
      """
    And the mock service received 1 request to "/expensive"
