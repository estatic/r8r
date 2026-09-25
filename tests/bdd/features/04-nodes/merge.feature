@spec-6.6 @phase-1 @node-merge
Feature: Merge node
  Merge v3 combines two inputs: append, combine by matching fields, combine
  by position, or choose one branch.

  Background:
    Given a workflow with nodes:
      | name      | type          | position |
      | Start     | manualTrigger | 0,0      |
      | Customers | code          | 200,-100 |
      | Orders    | code          | 200,100  |
      | Merge     | merge         | 400,0    |
    And the node "Customers" runs the JavaScript:
      """
      return [{ json: { id: 1, name: 'Ada' } }, { json: { id: 2, name: 'Grace' } }, { json: { id: 3, name: 'Linus' } }];
      """
    And the node "Orders" runs the JavaScript:
      """
      return [{ json: { id: 2, total: 20 } }, { json: { id: 1, total: 10 } }, { json: { id: 9, total: 90 } }];
      """
    And the connections:
      """
      Start -> Customers
      Start -> Orders
      Customers -> Merge:0
      Orders -> Merge:1
      """

  Scenario: Append outputs input 1's items, then input 2's
    Given the node "Merge" has parameters:
      """
      {"mode": "append"}
      """
    When I execute the workflow
    Then the node "Merge" outputs 6 items
    And the node "Merge" outputs items matching:
      """
      [{"name": "Ada"}, {"name": "Grace"}, {"name": "Linus"}, {"total": 20}, {"total": 10}, {"total": 90}]
      """

  Scenario: Combine by matching fields keeps matches by default
    Given the node "Merge" has parameters:
      """
      {"mode": "combine", "combineBy": "combineByFields", "fieldsToMatchString": "id", "options": {}}
      """
    When I execute the workflow
    Then the node "Merge" outputs:
      """
      [{"id": 1, "name": "Ada", "total": 10}, {"id": 2, "name": "Grace", "total": 20}]
      """

  Scenario: Combine by matching fields, keeping everything
    Given the node "Merge" has parameters:
      """
      {"mode": "combine", "combineBy": "combineByFields", "fieldsToMatchString": "id", "joinMode": "keepEverything", "options": {}}
      """
    When I execute the workflow
    Then the node "Merge" outputs 4 items

  Scenario: Combine by matching fields, keeping non-matches of input 1
    Given the node "Merge" has parameters:
      """
      {"mode": "combine", "combineBy": "combineByFields", "fieldsToMatchString": "id", "joinMode": "keepNonMatches", "outputDataFrom": "input1", "options": {}}
      """
    When I execute the workflow
    Then the node "Merge" outputs:
      """
      [{"id": 3, "name": "Linus"}]
      """

  Scenario: Combine by position pairs items by index
    Given the node "Merge" has parameters:
      """
      {"mode": "combine", "combineBy": "combineByPosition", "options": {}}
      """
    When I execute the workflow
    Then the node "Merge" outputs:
      """
      [{"id": 2, "name": "Ada", "total": 20}, {"id": 1, "name": "Grace", "total": 10}, {"id": 9, "name": "Linus", "total": 90}]
      """

  Scenario: Choose branch outputs one input's items after both arrive
    Given the node "Merge" has parameters:
      """
      {"mode": "chooseBranch", "chooseBranchMode": "waitForAll", "output": "specifiedInput", "useDataOfInput": 2}
      """
    When I execute the workflow
    Then the node "Merge" outputs:
      """
      [{"id": 2, "total": 20}, {"id": 1, "total": 10}, {"id": 9, "total": 90}]
      """
