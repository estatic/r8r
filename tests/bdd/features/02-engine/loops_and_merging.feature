@spec-2.4 @spec-6.2 @phase-1
Feature: Loops and multi-input nodes
  Cycles are legal. Loop Over Items (Split in Batches v3) feeds batches
  round a loop until the input is used up, then emits everything on its
  "done" output. Nodes with several inputs wait for their inputs as the
  execution order dictates.

  Scenario: Loop Over Items processes every batch and then reports done
    Given a workflow with nodes:
      | name    | type           | parameters                        |
      | Start   | manualTrigger  |                                   |
      | Loop    | splitInBatches | {"batchSize": 2, "options": {}}   |
      | Process | set            |                                   |
      | Done    | noOp           |                                   |
    And the node "Process" adds the fields:
      """
      {"processed": true}
      """
    And the connections:
      """
      Start -> Loop
      Loop:1 -> Process
      Process -> Loop
      Loop:0 -> Done
      """
    And the trigger outputs the items:
      """
      [{"n": 1}, {"n": 2}, {"n": 3}, {"n": 4}, {"n": 5}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Process" was executed 3 times
    And run 0 of the node "Process" outputs:
      """
      [{"n": 1, "processed": true}, {"n": 2, "processed": true}]
      """
    And run 2 of the node "Process" outputs:
      """
      [{"n": 5, "processed": true}]
      """
    And the node "Done" outputs:
      """
      [
        {"n": 1, "processed": true}, {"n": 2, "processed": true}, {"n": 3, "processed": true},
        {"n": 4, "processed": true}, {"n": 5, "processed": true}
      ]
      """

  Scenario: $runIndex counts loop iterations
    Given a workflow with nodes:
      | name    | type           | parameters                        |
      | Start   | manualTrigger  |                                   |
      | Loop    | splitInBatches | {"batchSize": 1, "options": {}}   |
      | Count   | set            |                                   |
    And the node "Count" adds the fields:
      """
      {"iteration": "={{ $runIndex }}"}
      """
    And the connections:
      """
      Start -> Loop
      Loop:1 -> Count
      Count -> Loop
      """
    And the trigger outputs the items:
      """
      [{"n": "a"}, {"n": "b"}, {"n": "c"}]
      """
    When I execute the workflow
    Then run 2 of the node "Count" outputs:
      """
      [{"n": "c", "iteration": 2}]
      """

  Scenario: An If splits items and a Merge reunites both branches
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Big?   | if            |
      | Big    | set           |
      | Small  | set           |
      | Merge  | merge         |
    And the node "Big?" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "c1", "leftValue": "={{ $json.amount }}", "rightValue": 100, "operator": {"type": "number", "operation": "gt"}}],
        "combinator": "and"}, "options": {}}
      """
    And the node "Big" adds the fields:
      """
      {"size": "big"}
      """
    And the node "Small" adds the fields:
      """
      {"size": "small"}
      """
    And the node "Merge" has parameters:
      """
      {"mode": "append"}
      """
    And the connections:
      """
      Start -> Big?
      Big?:0 -> Big
      Big?:1 -> Small
      Big -> Merge:0
      Small -> Merge:1
      """
    And the trigger outputs the items:
      """
      [{"amount": 50}, {"amount": 500}, {"amount": 20}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Merge" outputs:
      """
      [{"amount": 500, "size": "big"}, {"amount": 50, "size": "small"}, {"amount": 20, "size": "small"}]
      """
    And the node "Merge" was executed 1 time

  Scenario: A Merge waits for both inputs even when one branch is longer
    Given a workflow with nodes:
      | name   | type          | position |
      | Start  | manualTrigger | 0,0      |
      | Short  | set           | 200,-100 |
      | Long 1 | noOp          | 200,100  |
      | Long 2 | set           | 400,100  |
      | Merge  | merge         | 600,0    |
    And the node "Short" sets the fields:
      """
      {"from": "short"}
      """
    And the node "Long 2" sets the fields:
      """
      {"from": "long"}
      """
    And the node "Merge" has parameters:
      """
      {"mode": "append"}
      """
    And the connections:
      """
      Start -> Short
      Start -> Long 1
      Long 1 -> Long 2
      Short -> Merge:0
      Long 2 -> Merge:1
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Merge" was executed 1 time
    And the node "Merge" outputs:
      """
      [{"from": "short"}, {"from": "long"}]
      """
