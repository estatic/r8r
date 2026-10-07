@spec-6.8 @phase-5
Feature: Nodes as AI tools and $fromAI
  Every node n8n marks `usableAsTool` also exists as a tool sub-node,
  `<type>Tool` (e.g. `n8n-nodes-base.httpRequestTool`). Connected to an
  agent over `ai_tool`, it is offered to the model as a function named
  after the node, whose arguments are the `$fromAI(key, description,
  type, default)` calls in its parameters. When the model calls it, the
  node runs once with the model's arguments as its input item, so
  `$fromAI('key')` resolves to the model's value (or the default), and
  the model gets the node's output JSON back.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: An HTTP Request tool gets its query from the model through $fromAI
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Weather", "arguments": "{\"city\": \"Paris\"}"}}]},
        {"role": "assistant", "content": "It is 21 degrees in Paris."}
      ]
      """
    And the mock service responds to GET "/weather" with status 200 and body:
      """
      {"temp": 21}
      """
    And a workflow with nodes:
      | name    | type            | parameters                                                                                                                                                                                                 |
      | Start   | manualTrigger   |                                                                                                                                                                                                            |
      | Agent   | lc.agent        | {"promptType": "define", "text": "How warm is it in Paris?", "options": {}}                                                                                                                                |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                                                                           |
      | Weather | httpRequestTool | {"url": "=%{MOCK_URL}/weather?city={{ $fromAI('city', 'The city to look up', 'string') }}&units={{ $fromAI('units', 'Unit system', 'string', 'metric') }}", "toolDescription": "Current weather for a city", "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Weather -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "It is 21 degrees in Paris."}]
      """
    And chat request 1 offers the tool "Weather" as:
      """
      {
        "name": "Weather",
        "description": "Current weather for a city",
        "parameters": {
          "type": "object",
          "properties": {
            "city": {"type": "string", "description": "The city to look up"},
            "units": {"type": "string", "description": "Unit system", "default": "metric"}
          }
        }
      }
      """
    And the last request to "/weather" had the query parameter "city" equal to "Paris"
    And the last request to "/weather" had the query parameter "units" equal to "metric"
    And chat request 2 contains a "tool" message containing "21"
    And the node "Weather" has run data on the "ai_tool" connection

  Scenario: Without a manual description a tool is described by its node type
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "ok"}]
      """
    And a workflow with nodes:
      | name     | type            | parameters                                                                        |
      | Start    | manualTrigger   |                                                                                   |
      | Agent    | lc.agent        | {"promptType": "define", "text": "hi", "options": {}}                             |
      | Model    | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}  |
      | Get page | httpRequestTool | {"url": "={{ $fromAI('url') }}", "options": {}}                                   |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Get page -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 offers the tool "Get_page" as:
      """
      {
        "name": "Get_page",
        "description": "Makes an HTTP request and returns the response data",
        "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}
      }
      """

  Scenario: Any native node can be a tool and returns its output JSON
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Hash", "arguments": "{\"text\": \"hello\"}"}}]},
        {"role": "assistant", "content": "done"}
      ]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                                                    |
      | Start | manualTrigger   |                                                                                                               |
      | Agent | lc.agent        | {"promptType": "define", "text": "Hash hello", "options": {}}                                                 |
      | Model | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                              |
      | Hash  | cryptoTool      | {"action": "hash", "type": "SHA256", "value": "={{ $fromAI('text', 'Text to hash') }}", "dataPropertyName": "digest"} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Hash -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 2 contains a "tool" message containing "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    And the node "Hash" has run data on the "ai_tool" connection

  # n8n records the tool's error in its run data but gives the model an
  # empty tool result, and the agent carries on.
  Scenario: A failing tool node hands the model an empty result
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Weather", "arguments": "{\"city\": \"Paris\"}"}}]},
        {"role": "assistant", "content": "The weather service is down."}
      ]
      """
    And the mock service responds to GET "/weather" with status 500 and body:
      """
      {"error": "boom"}
      """
    And a workflow with nodes:
      | name    | type            | parameters                                                                                            |
      | Start   | manualTrigger   |                                                                                                       |
      | Agent   | lc.agent        | {"promptType": "define", "text": "How warm is it in Paris?", "options": {}}                           |
      | Model   | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                      |
      | Weather | httpRequestTool | {"url": "=%{MOCK_URL}/weather?city={{ $fromAI('city', 'The city', 'string') }}", "options": {}}       |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Weather -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "The weather service is down."}]
      """
    And chat request 2 has a "tool" message with the content ""
    And the node "Weather" has run data on the "ai_tool" connection
