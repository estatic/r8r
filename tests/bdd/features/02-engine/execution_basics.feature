@spec-2.4 @spec-6.2 @phase-1
Feature: Items flow between nodes and every run is recorded
  Data moves as arrays of items `{ json, binary?, pairedItem? }`. The result
  of an execution is n8n's IRun: `status`, `mode`, and
  `data.resultData.runData[nodeName][runIndex]` with timing, source, data
  and error, which the editor replays.

  Scenario: A manual trigger emits one empty item
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
    When I execute the workflow
    Then the execution succeeds
    And the node "Start" outputs:
      """
      [{}]
      """

  Scenario: Items pass through a chain and each node transforms every item
    Given a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Enrich  | set           |
    And the node "Enrich" adds the fields:
      """
      {"source": "crm"}
      """
    And the connections "Start -> Enrich"
    And the trigger outputs the items:
      """
      [{"id": 1}, {"id": 2}, {"id": 3}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Enrich" outputs:
      """
      [{"id": 1, "source": "crm"}, {"id": 2, "source": "crm"}, {"id": 3, "source": "crm"}]
      """
    And the last node executed is "Enrich"

  Scenario: Every node run records timing, status, source and data
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Pass   | noOp          |
    And the connections "Start -> Pass"
    When I execute the workflow
    Then the execution succeeds
    And every run of the node "Start" records its timing, status and source
    And every run of the node "Pass" records its timing, status and source
    And the node "Pass" received its input from "Start"
    And the execution result matches:
      """
      {
        "status": "success",
        "finished": true,
        "mode": "$string",
        "startedAt": "$datetime",
        "stoppedAt": "$datetime",
        "data": {"resultData": {"lastNodeExecuted": "Pass", "runData": {"Start": [{"executionStatus": "success"}], "Pass": [{"executionStatus": "success"}]}}}
      }
      """

  Scenario: Output items are paired with the input items they came from
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Tag    | set           |
    And the node "Tag" adds the fields:
      """
      {"tagged": true}
      """
    And the connections "Start -> Tag"
    And the trigger outputs the items:
      """
      [{"n": 1}, {"n": 2}]
      """
    When I execute the workflow
    Then the items of the node "Tag" are paired as:
      | item | pairedItem  |
      | 0    | {"item": 0} |
      | 1    | {"item": 1} |

  Scenario: A node that fans out keeps paired items pointing at their source
    Given a workflow with nodes:
      | name   | type          | parameters                                                       |
      | Start  | manualTrigger |                                                                  |
      | Split  | splitOut      | {"fieldToSplitOut": "lines", "include": "noOtherFields", "options": {}} |
    And the connections "Start -> Split"
    And the trigger outputs the items:
      """
      [{"lines": [{"sku": "a"}, {"sku": "b"}]}, {"lines": [{"sku": "c"}]}]
      """
    When I execute the workflow
    Then the node "Split" outputs:
      """
      [{"sku": "a"}, {"sku": "b"}, {"sku": "c"}]
      """
    And the items of the node "Split" are paired as:
      | item | pairedItem  |
      | 0    | {"item": 0} |
      | 1    | {"item": 0} |
      | 2    | {"item": 1} |

  Scenario: A branch whose node outputs no items stops there
    Given a workflow with nodes:
      | name   | type          | parameters |
      | Start  | manualTrigger |            |
      | Keep   | filter        | {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2}, "conditions": [{"id": "c1", "leftValue": "={{ $json.keep }}", "rightValue": "", "operator": {"type": "boolean", "operation": "true", "singleValue": true}}], "combinator": "and"}, "options": {}} |
      | After  | noOp          |            |
    And the connections "Start -> Keep -> After"
    And the trigger outputs the items:
      """
      [{"keep": false}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Keep" outputs 0 items
    And the node "After" was not executed

  Scenario: A failing node fails the execution and records the error on that node
    Given a workflow with nodes:
      | name  | type          | parameters                                              |
      | Start | manualTrigger |                                                         |
      | Boom  | stopAndError  | {"errorType": "errorMessage", "errorMessage": "Boom!"}  |
      | Never | noOp          |                                                         |
    And the connections "Start -> Boom -> Never"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Boom!"
    And the node "Boom" failed with an error containing "Boom!"
    And the node "Never" was not executed
    And the last node executed is "Boom"
    And the command fails

  Scenario: Binary data travels with its item
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Make     | code          |
      | Describe | set           |
    And the node "Make" runs the JavaScript:
      """
      const data = Buffer.from('hello world').toString('base64');
      return [{ json: { name: 'greeting' }, binary: { file: { data, mimeType: 'text/plain', fileName: 'hello.txt' } } }];
      """
    And the node "Describe" sets the fields:
      """
      {"mime": "={{ $binary.file.mimeType }}", "fileName": "={{ $binary.file.fileName }}"}
      """
    And the connections "Start -> Make -> Describe"
    When I execute the workflow
    Then the execution succeeds
    And the node "Describe" outputs:
      """
      [{"mime": "text/plain", "fileName": "hello.txt"}]
      """
