@spec-6.8 @phase-5
Feature: Workflow Tool
  `@n8n/n8n-nodes-langchain.toolWorkflow` (v2) lets an agent run another
  workflow. It is named after the node (from v2.2) and described by its
  `description`. When the sub-workflow's inputs are mapped with
  `$fromAI(...)`, the model fills them in as the tool's arguments;
  otherwise the tool takes one string, which the sub-workflow receives as
  `query`. The model gets the sub-workflow's last items as JSON, or
  `There was an error: "..."` when it fails, and the agent carries on.

  Background:
    Given a running r8r server with an owner and an API key
    And a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: Workflow inputs mapped with $fromAI become the tool's arguments
    Given a workflow named "Weather child" with nodes:
      | name   | type                   | parameters                     |
      | Input  | executeWorkflowTrigger | {"inputSource": "passthrough"} |
      | Answer | set                    |                                |
    And the node "Answer" sets the fields:
      """
      {"answer": "={{ 'Sunny in ' + $json.city }}"}
      """
    And the connections "Input -> Answer"
    And the workflow setting "callerPolicy" is "any"
    And the workflow "Weather child" is active
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Weather", "arguments": "{\"city\": \"Paris\"}"}}]},
        {"role": "assistant", "content": "It is sunny in Paris."}
      ]
      """
    And a workflow named "Agent parent" with nodes:
      | name    | type            | parameters                                                                          |
      | Webhook | webhook         | {"httpMethod": "POST", "path": "ask", "responseMode": "lastNode", "options": {}}    |
      | Agent   | lc.agent        | {"promptType": "define", "text": "How is the weather in Paris?", "options": {}}     |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}    |
      | Weather | lc.toolWorkflow |                                                                                     |
    And the node "Weather" has parameters:
      """
      {
        "description": "Weather for a city",
        "source": "database",
        "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Weather child}", "mode": "id"},
        "workflowInputs": {
          "mappingMode": "defineBelow",
          "value": {"city": "={{ $fromAI('city', 'The city', 'string') }}"},
          "matchingColumns": [],
          "schema": [{"id": "city", "displayName": "city", "required": false, "defaultMatch": false, "display": true, "canBeUsedToMatch": true, "type": "string", "removed": false}],
          "attemptToConvertTypes": false,
          "convertFieldsToString": false
        }
      }
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Webhook -> Agent
      Model -[ai_languageModel]-> Agent
      Weather -[ai_tool]-> Agent
      """
    And the workflow "Agent parent" is active
    When I send a POST request to "/webhook/ask" with body:
      """
      {}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"output": "It is sunny in Paris."}
      """
    And chat request 1 offers the tool "Weather" as:
      """
      {"name": "Weather", "description": "Weather for a city", "parameters": {"type": "object", "properties": {"city": {"type": "string", "description": "The city"}}, "required": ["city"]}}
      """
    And chat request 2 contains a "tool" message containing "Sunny in Paris"
    And the workflow "Weather child" has 1 execution

  Scenario: Without mapped inputs the tool takes a string the sub-workflow gets as query
    Given a workflow named "Echo child" with nodes:
      | name   | type                   | parameters                     |
      | Input  | executeWorkflowTrigger | {"inputSource": "passthrough"} |
      | Answer | set                    |                                |
    And the node "Answer" sets the fields:
      """
      {"answer": "={{ 'You said ' + $json.query }}"}
      """
    And the connections "Input -> Answer"
    And the workflow setting "callerPolicy" is "any"
    And the workflow "Echo child" is active
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Echo", "arguments": "{\"input\": \"hi there\"}"}}]},
        {"role": "assistant", "content": "done"}
      ]
      """
    And a workflow named "Agent parent" with nodes:
      | name    | type            | parameters                                                                          |
      | Webhook | webhook         | {"httpMethod": "POST", "path": "ask", "responseMode": "lastNode", "options": {}}    |
      | Agent   | lc.agent        | {"promptType": "define", "text": "Echo hi there", "options": {}}                    |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}    |
      | Echo    | lc.toolWorkflow |                                                                                     |
    And the node "Echo" has parameters:
      """
      {"description": "Echoes the input", "source": "database", "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Echo child}", "mode": "id"}, "workflowInputs": {"mappingMode": "defineBelow", "value": {}, "matchingColumns": [], "schema": [], "attemptToConvertTypes": false, "convertFieldsToString": false}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Webhook -> Agent
      Model -[ai_languageModel]-> Agent
      Echo -[ai_tool]-> Agent
      """
    And the workflow "Agent parent" is active
    When I send a POST request to "/webhook/ask" with body:
      """
      {}
      """
    Then the response status is 200
    And chat request 1 offers the tool "Echo" as:
      """
      {"name": "Echo", "description": "Echoes the input", "parameters": {"type": "object", "properties": {"input": {"type": "string"}}}}
      """
    And chat request 2 contains a "tool" message containing "You said hi there"

  Scenario: A failing sub-workflow is reported to the model
    Given a workflow named "Failing child" with nodes:
      | name  | type                   | parameters                                  |
      | Input | executeWorkflowTrigger | {"inputSource": "passthrough"}              |
      | Fail  | stopAndError           | {"errorMessage": "the weather is broken"}   |
    And the connections "Input -> Fail"
    And the workflow setting "callerPolicy" is "any"
    And the workflow "Failing child" is active
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Weather", "arguments": "{\"input\": \"Paris\"}"}}]},
        {"role": "assistant", "content": "The weather service failed."}
      ]
      """
    And a workflow named "Agent parent" with nodes:
      | name    | type            | parameters                                                                          |
      | Webhook | webhook         | {"httpMethod": "POST", "path": "ask", "responseMode": "lastNode", "options": {}}    |
      | Agent   | lc.agent        | {"promptType": "define", "text": "Weather in Paris?", "options": {}}                 |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}    |
      | Weather | lc.toolWorkflow |                                                                                     |
    And the node "Weather" has parameters:
      """
      {"description": "Weather", "source": "database", "workflowId": {"__rl": true, "value": "%{WORKFLOW_ID:Failing child}", "mode": "id"}, "workflowInputs": {"mappingMode": "defineBelow", "value": {}, "matchingColumns": [], "schema": [], "attemptToConvertTypes": false, "convertFieldsToString": false}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Webhook -> Agent
      Model -[ai_languageModel]-> Agent
      Weather -[ai_tool]-> Agent
      """
    And the workflow "Agent parent" is active
    When I send a POST request to "/webhook/ask" with body:
      """
      {}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"output": "The weather service failed."}
      """
    And chat request 2 contains a "tool" message containing "There was an error"
    And chat request 2 contains a "tool" message containing "the weather is broken"
