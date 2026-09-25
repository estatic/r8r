@spec-2.3 @spec-6.1 @spec-4.3 @phase-1
Feature: n8n workflow JSON is a frozen contract
  r8r imports, runs and exports n8n workflow JSON unchanged (goal G1): node
  names key the connections, unknown fields survive a round trip, key order
  is kept on export, and cycles or unknown node types do not stop a save.

  Background:
    Given the file "order-flow.json" contains:
      """
      {
        "id": "wf-order-flow",
        "name": "Order flow",
        "nodes": [
          {
            "parameters": {},
            "id": "0f5532f9-36ba-4bef-86c7-30d607400b15",
            "name": "When clicking 'Execute workflow'",
            "type": "n8n-nodes-base.manualTrigger",
            "typeVersion": 1,
            "position": [0, 0]
          },
          {
            "parameters": {
              "mode": "manual",
              "assignments": {"assignments": [{"id": "a1", "name": "status", "value": "new", "type": "string"}]},
              "includeOtherFields": false,
              "options": {}
            },
            "id": "7a1e8f0e-0f7e-4c63-9b0f-4a1c2b5f4e11",
            "name": "Mark new",
            "type": "n8n-nodes-base.set",
            "typeVersion": 3.4,
            "position": [220, 0],
            "notes": "sets the initial status",
            "notesInFlow": true,
            "x-custom-node-field": {"kept": true}
          }
        ],
        "connections": {
          "When clicking 'Execute workflow'": {
            "main": [[{"node": "Mark new", "type": "main", "index": 0}]]
          }
        },
        "settings": {"executionOrder": "v1", "saveManualExecutions": true, "callerPolicy": "workflowsFromSameOwner"},
        "staticData": null,
        "pinData": {},
        "meta": {"templateCredsSetupCompleted": true, "instanceId": "abc123"},
        "tags": [],
        "x-custom-top-level": {"answer": 42}
      }
      """

  Scenario: A workflow exported from n8n runs unchanged
    When I run "r8r execute --file=order-flow.json --rawOutput"
    Then the execution succeeds
    And the node "Mark new" outputs:
      """
      [{"status": "new"}]
      """

  Scenario: Import then export preserves every field, including unknown ones
    Given I successfully run "r8r import:workflow --input=order-flow.json"
    When I run "r8r export:workflow --id=wf-order-flow --output=exported.json"
    Then the command succeeds
    And the file "exported.json" contains JSON matching:
      """
      {
        "id": "wf-order-flow",
        "name": "Order flow",
        "nodes": [
          {"name": "When clicking 'Execute workflow'", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0]},
          {
            "name": "Mark new",
            "typeVersion": 3.4,
            "notes": "sets the initial status",
            "notesInFlow": true,
            "x-custom-node-field": {"kept": true},
            "parameters": {
              "mode": "manual",
              "assignments": {"assignments": [{"id": "a1", "name": "status", "value": "new", "type": "string"}]},
              "includeOtherFields": false,
              "options": {}
            }
          }
        ],
        "connections": {
          "When clicking 'Execute workflow'": {"main": [[{"node": "Mark new", "type": "main", "index": 0}]]}
        },
        "settings": {"executionOrder": "v1", "saveManualExecutions": true, "callerPolicy": "workflowsFromSameOwner"},
        "meta": {"templateCredsSetupCompleted": true, "instanceId": "abc123"},
        "x-custom-top-level": {"answer": 42}
      }
      """

  Scenario: Export keeps the key order of nodes and parameters
    Given I successfully run "r8r import:workflow --input=order-flow.json"
    When I run "r8r export:workflow --id=wf-order-flow --output=exported.json"
    Then the object at "nodes[1]" in the file "exported.json" has the keys in the order "parameters, id, name, type, typeVersion, position"
    And the object at "nodes[1].parameters" in the file "exported.json" has the keys in the order "mode, assignments, includeOtherFields, options"

  Scenario: Exporting all workflows returns an array
    Given I successfully run "r8r import:workflow --input=order-flow.json"
    When I run "r8r export:workflow --all --output=all.json"
    Then the command succeeds
    And the file "all.json" contains JSON matching:
      """
      [{"id": "wf-order-flow", "name": "Order flow"}]
      """

  Scenario: Connections are keyed by the source node's name
    Given a workflow with nodes:
      | name          | type          |
      | Start         | manualTrigger |
      | Say hello     | set           |
    And the node "Say hello" sets the fields:
      """
      {"greeting": "hello"}
      """
    And the connections "Start -> Say hello"
    When I execute the workflow
    Then the execution succeeds
    And the node "Say hello" received its input from "Start"
    And the node "Say hello" outputs:
      """
      [{"greeting": "hello"}]
      """

  Scenario: A workflow with a cycle is valid and can be imported
    Given the file "loop.json" contains:
      """
      {
        "id": "wf-loop",
        "name": "Loop",
        "nodes": [
          {"parameters": {}, "id": "1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0]},
          {"parameters": {"batchSize": 1, "options": {}}, "id": "2", "name": "Loop", "type": "n8n-nodes-base.splitInBatches", "typeVersion": 3, "position": [200, 0]},
          {"parameters": {}, "id": "3", "name": "Body", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [400, 0]}
        ],
        "connections": {
          "Start": {"main": [[{"node": "Loop", "type": "main", "index": 0}]]},
          "Loop": {"main": [[], [{"node": "Body", "type": "main", "index": 0}]]},
          "Body": {"main": [[{"node": "Loop", "type": "main", "index": 0}]]}
        },
        "settings": {"executionOrder": "v1"}
      }
      """
    When I run "r8r import:workflow --input=loop.json"
    Then the command succeeds

  Scenario: A workflow with an unknown node type can be saved but not executed
    Given the file "unknown.json" contains:
      """
      {
        "id": "wf-unknown",
        "name": "Unknown node",
        "nodes": [
          {"parameters": {}, "id": "1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0]},
          {"parameters": {"foo": "bar"}, "id": "2", "name": "Mystery", "type": "n8n-nodes-community.doesNotExist", "typeVersion": 1, "position": [200, 0]}
        ],
        "connections": {"Start": {"main": [[{"node": "Mystery", "type": "main", "index": 0}]]}},
        "settings": {}
      }
      """
    When I run "r8r import:workflow --input=unknown.json"
    Then the command succeeds
    When I run "r8r execute --file=unknown.json --rawOutput"
    Then the execution fails
    And the execution error message contains "n8n-nodes-community.doesNotExist"

  Scenario: Nodes that are not connected to the start node do not run
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Attached | noOp          |
      | Orphan   | noOp          |
    And the connections "Start -> Attached"
    When I execute the workflow
    Then the execution succeeds
    And the node "Attached" was executed 1 time
    And the node "Orphan" was not executed

  Scenario: Sticky notes are kept but never executed
    Given a workflow with nodes:
      | name  | type          | parameters                      |
      | Start | manualTrigger |                                 |
      | Note  | stickyNote    | {"content": "## Read me first"} |
    When I execute the workflow
    Then the execution succeeds
    And the node "Note" was not executed
