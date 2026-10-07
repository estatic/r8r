@spec-6.8 @phase-5
Feature: Code Tool
  `@n8n/n8n-nodes-langchain.toolCode` lets an agent call JavaScript. The
  tool is named after the node (from v1.2) and described by its
  `description`. Without an input schema it takes one string, which the
  code sees as `query`; with "Specify Input Schema" it takes an object
  (from a JSON example or a JSON Schema) and `query` is that object. The
  code's return value is the tool's answer: a string, or a number turned
  into one. Anything else, or a thrown error, reaches the model as
  `There was an error: "..."`.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: The model's string input reaches the code as query
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Shout", "arguments": "{\"input\": \"hello\"}"}}]},
        {"role": "assistant", "content": "HELLO"}
      ]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                                              |
      | Start | manualTrigger   |                                                                                                         |
      | Agent | lc.agent        | {"promptType": "define", "text": "Shout hello", "options": {}}                                          |
      | Model | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                        |
      | Shout | lc.toolCode     | {"description": "Upper-cases the input", "jsCode": "return query.toUpperCase();"}                       |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Shout -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "HELLO"}]
      """
    And chat request 1 offers the tool "Shout" as:
      """
      {"name": "Shout", "description": "Upper-cases the input", "parameters": {"type": "object", "properties": {"input": {"type": "string"}}}}
      """
    And chat request 2 has a "tool" message with the content "HELLO"
    And the node "Shout" has run data on the "ai_tool" connection

  Scenario: With an input schema from a JSON example, query is the arguments object
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Add", "arguments": "{\"a\": 2, \"b\": 3}"}}]},
        {"role": "assistant", "content": "5"}
      ]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                                                                                                                   |
      | Start | manualTrigger   |                                                                                                                                                                              |
      | Agent | lc.agent        | {"promptType": "define", "text": "Add 2 and 3", "options": {}}                                                                                                               |
      | Model | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                                             |
      | Add   | lc.toolCode     |                                                                                                                                                                              |
    And the node "Add" has parameters:
      """
      {"description": "Adds two numbers", "jsCode": "return query.a + query.b;", "specifyInputSchema": true, "schemaType": "fromJson", "jsonSchemaExample": "{\"a\": 1, \"b\": 2}"}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Add -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 offers the tool "Add" as:
      """
      {"name": "Add", "parameters": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}}
      """
    And chat request 2 has a "tool" message with the content "5"

  Scenario: A wrong return type is reported to the model
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Broken", "arguments": "{\"input\": \"x\"}"}}]},
        {"role": "assistant", "content": "sorry"}
      ]
      """
    And a workflow with nodes:
      | name   | type            | parameters                                                                       |
      | Start  | manualTrigger   |                                                                                  |
      | Agent  | lc.agent        | {"promptType": "define", "text": "go", "options": {}}                            |
      | Model  | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
      | Broken | lc.toolCode     | {"description": "Returns an object", "jsCode": "return {a: 1};"}                 |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Broken -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 2 contains a "tool" message containing "There was an error"
    And chat request 2 contains a "tool" message containing "Wrong output type returned"

  Scenario: A thrown error is reported to the model
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Thrower", "arguments": "{\"input\": \"x\"}"}}]},
        {"role": "assistant", "content": "sorry"}
      ]
      """
    And a workflow with nodes:
      | name    | type            | parameters                                                                       |
      | Start   | manualTrigger   |                                                                                  |
      | Agent   | lc.agent        | {"promptType": "define", "text": "go", "options": {}}                            |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
      | Thrower | lc.toolCode     | {"description": "Throws", "jsCode": "throw new Error('boom');"}                  |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Thrower -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 2 contains a "tool" message containing "There was an error"
    And chat request 2 contains a "tool" message containing "boom"
