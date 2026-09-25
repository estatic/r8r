@n8n-compat
Feature: n8n workflow JSON compatibility (spec 2.3, 4.3, 6.1)
  r8n imports, stores and exports n8n workflow JSON unchanged: connections
  keyed by source node name, typeVersion, settings, pinData and any field it
  does not understand.

  Background:
    Given an r8n instance with an owner account
    And I have a public API key

  Scenario: A workflow round-trips through import and export without loss
    Given the n8n workflow "roundtrip":
      """
      {"nodes": [
        {"id": "1", "name": "When clicking 'Test workflow'", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
        {"id": "2", "name": "Edit Fields", "type": "n8n-nodes-base.set", "typeVersion": 3.4, "position": [220, 0],
         "parameters": {"assignments": {"assignments": [{"id": "x", "name": "a", "value": 1, "type": "number"}]}, "options": {}},
         "notes": "keep me", "retryOnFail": true, "maxTries": 3, "waitBetweenTries": 1000, "onError": "continueRegularOutput"}
       ],
       "connections": {"When clicking 'Test workflow'": {"main": [[{"node": "Edit Fields", "type": "main", "index": 0}]]}},
       "settings": {"executionOrder": "v1", "timezone": "Europe/Warsaw", "saveManualExecutions": true}}
      """
    When I export the workflow "roundtrip"
    Then the JSON at "/nodes/1/typeVersion" is 3.4
    And the JSON at "/nodes/1/notes" is "keep me"
    And the JSON at "/nodes/1/onError" is "continueRegularOutput"
    And the JSON at "/nodes/1/maxTries" is 3
    And the JSON at "/connections" equals:
      """
      {"When clicking 'Test workflow'": {"main": [[{"node": "Edit Fields", "type": "main", "index": 0}]]}}
      """
    And the JSON at "/settings/timezone" is "Europe/Warsaw"

  Scenario: Unknown top-level and node fields are preserved
    Given the n8n workflow "extras":
      """
      {"nodes": [{"id": "1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {},
                  "futureNodeField": {"x": 1}}],
       "connections": {}, "futureTopLevel": ["kept"]}
      """
    When I export the workflow "extras"
    Then the JSON at "/futureTopLevel" equals:
      """
      ["kept"]
      """
    And the JSON at "/nodes/0/futureNodeField/x" is 1

  Scenario: AI connection types are preserved
    Given the n8n workflow "ai-links":
      """
      {"nodes": [
        {"id": "1", "name": "AI Agent", "type": "@n8n/n8n-nodes-langchain.agent", "typeVersion": 1.7, "position": [0, 0], "parameters": {}},
        {"id": "2", "name": "OpenAI Chat Model", "type": "@n8n/n8n-nodes-langchain.lmChatOpenAi", "typeVersion": 1.2, "position": [0, 200], "parameters": {}}
       ],
       "connections": {"OpenAI Chat Model": {"ai_languageModel": [[{"node": "AI Agent", "type": "ai_languageModel", "index": 0}]]}}}
      """
    When I export the workflow "ai-links"
    Then the JSON at "/connections/OpenAI Chat Model/ai_languageModel/0/0/node" is "AI Agent"

  Scenario: Cycles are allowed (loops are legal)
    When I import the n8n workflow:
      """
      {"name": "loop", "nodes": [
        {"id": "1", "name": "A", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [0, 0], "parameters": {}},
        {"id": "2", "name": "B", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [200, 0], "parameters": {}}
       ],
       "connections": {"A": {"main": [[{"node": "B", "type": "main", "index": 0}]]}, "B": {"main": [[{"node": "A", "type": "main", "index": 0}]]}},
       "settings": {}}
      """
    Then the response status is 200

  Scenario: Unknown node types are accepted but flagged
    When I import the n8n workflow:
      """
      {"name": "unknown", "nodes": [{"id": "1", "name": "Mystery", "type": "n8n-nodes-community.doesNotExist", "typeVersion": 1, "position": [0, 0], "parameters": {}}],
       "connections": {}, "settings": {}}
      """
    Then the response status is 200
    And the response body contains "doesNotExist"

  Scenario Outline: Structurally invalid workflows are rejected
    When I import the n8n workflow:
      """
      <workflow>
      """
    Then the response status is 400

    Examples:
      | workflow                                                                                                                                             |
      | {"nodes": [], "connections": {}, "settings": {}}                                                                                                     |
      | {"name": "no-nodes-key", "connections": {}, "settings": {}}                                                                                          |
      | {"name": "dangling", "nodes": [], "connections": {"Ghost": {"main": [[{"node": "Nobody", "type": "main", "index": 0}]]}}, "settings": {}}             |
      | {"name": "dup-names", "nodes": [{"id": "1", "name": "A", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [0,0], "parameters": {}}, {"id": "2", "name": "A", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [0,0], "parameters": {}}], "connections": {}, "settings": {}} |

  Scenario: Renaming a node updates connections and expressions that reference it
    Given the n8n workflow "rename":
      """
      {"nodes": [
        {"id": "1", "name": "Old Name", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
        {"id": "2", "name": "Use", "type": "n8n-nodes-base.set", "typeVersion": 3.4, "position": [200, 0],
         "parameters": {"mode": "raw", "jsonOutput": "={{ { v: $('Old Name').item.json.x } }}", "options": {}}}
       ],
       "connections": {"Old Name": {"main": [[{"node": "Use", "type": "main", "index": 0}]]}}}
      """
    When I send a PATCH request to "/rest/workflows/{rename}/nodes/Old Name/rename" with JSON:
      """
      {"newName": "New Name"}
      """
    And I export the workflow "rename"
    Then the JSON at "/connections/New Name/main/0/0/node" is "Use"
    And there is no JSON at "/connections/Old Name"
    And the response body contains "$('New Name')"

  Scenario: Reading a workflow requires a valid API key
    Given I use the API key "not-a-key"
    When I send a GET request to "/api/v1/workflows"
    Then the response status is 401
