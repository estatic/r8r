@spec-6.8 @phase-5
Feature: Anthropic chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatAnthropic` plugs into the same
  `ai_languageModel` abstraction as the OpenAI Chat Model (see
  `agents.feature`), talking to Anthropic's Messages API
  (`POST /v1/messages`) instead of an OpenAI-compatible endpoint: a
  top-level `system` field, `x-api-key`/`anthropic-version` headers, a
  required `max_tokens`, and `tool_use`/`tool_result` content blocks for
  tool calls. The model here is a mock Anthropic server.

  Background:
    Given a mock HTTP service
    And the credential "Mock Anthropic" of type "anthropicApi" with the data:
      """
      {"apiKey": "sk-ant-test-123", "url": "%{MOCK_URL}"}
      """

  Scenario: An agent answers with Anthropic, using the Messages API shape
    Given a mock Anthropic API that replies in order:
      """
      [{"content": [{"type": "text", "text": "Paris"}], "stop_reason": "end_turn"}]
      """
    And a workflow with nodes:
      | name  | type              | parameters                                                                                                                    |
      | Start | manualTrigger     |                                                                                                                               |
      | Agent | lc.agent          | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatAnthropic | {"model": "claude-3-5-sonnet-20241022", "options": {"maxTokensToSample": 512, "temperature": 0.2}}                          |
    And the node "Model" uses the "anthropicApi" credential "Mock Anthropic"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    And the trigger outputs the items:
      """
      [{"country": "France"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "Paris"}]
      """
    And the last request to "/v1/messages" had a JSON body matching:
      """
      {
        "model": "claude-3-5-sonnet-20241022",
        "max_tokens": 512,
        "temperature": 0.2,
        "system": "Answer concisely.",
        "messages": [
          {"role": "user", "content": "What is the capital of France?"}
        ]
      }
      """
    And the last request to "/v1/messages" had the header "x-api-key" equal to "sk-ant-test-123"
    And the last request to "/v1/messages" had the header "anthropic-version" equal to "2023-06-01"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns Anthropic's answer
    Given a mock Anthropic API that replies in order:
      """
      [{"content": [{"type": "text", "text": "Paris"}]}]
      """
    And a workflow with nodes:
      | name  | type               | parameters                                                                       |
      | Start | manualTrigger      |                                                                                  |
      | Chain | lc.chainLlm        | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatAnthropic | {"model": "claude-3-5-sonnet-20241022", "options": {}}                          |
    And the node "Model" uses the "anthropicApi" credential "Mock Anthropic"
    And the connections:
      """
      Start -> Chain
      Model -[ai_languageModel]-> Chain
      """
    And the trigger outputs the items:
      """
      [{"country": "France"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Chain" outputs:
      """
      [{"text": "Paris"}]
      """
    And anthropic chat request 1 contains a "user" message containing "capital of France"
    And the last request to "/v1/messages" had a JSON body matching:
      """
      {"max_tokens": 4096}
      """

  Scenario: An agent calls a tool through Anthropic and answers with its result
    Given a mock Anthropic API that replies in order:
      """
      [
        {"content": [{"type": "tool_use", "id": "toolu_1", "name": "Calculator", "input": {"input": "6 * 7"}}], "stop_reason": "tool_use"},
        {"content": [{"type": "text", "text": "The answer is 42."}], "stop_reason": "end_turn"}
      ]
      """
    And a workflow with nodes:
      | name       | type               | parameters                                                                                           |
      | Start      | manualTrigger      |                                                                                                      |
      | Agent      | lc.agent           | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatAnthropic | {"model": "claude-3-5-sonnet-20241022", "options": {}}                                               |
      | Calculator | lc.toolCalculator  |                                                                                                      |
    And the node "Model" uses the "anthropicApi" credential "Mock Anthropic"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Calculator -[ai_tool]-> Agent
      """
    And the trigger outputs the items:
      """
      [{"question": "What is six times seven?"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "The answer is 42."}]
      """
    And the mock Anthropic API received 2 chat requests
    And anthropic chat request 1 offers the tool "Calculator"
    And anthropic chat request 1 has the system prompt "Be exact."
    And anthropic chat request 2 contains a "user" message containing "42"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: A provider error surfaces on the agent without leaking the API key
    Given the mock service responds to POST "/v1/messages" with status 401 and body:
      """
      {"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}
      """
    And a workflow with nodes:
      | name  | type               | parameters                                             |
      | Start | manualTrigger      |                                                        |
      | Agent | lc.agent           | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatAnthropic | {"model": "claude-3-5-sonnet-20241022", "options": {}} |
    And the node "Model" uses the "anthropicApi" credential "Mock Anthropic"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "invalid x-api-key"
    And the execution data does not contain "sk-ant-test-123"

  Scenario: A missing model surfaces Anthropic's not_found error
    Given the mock service responds to POST "/v1/messages" with status 404 and body:
      """
      {"type": "error", "error": {"type": "not_found_error", "message": "model: bogus-model"}}
      """
    And a workflow with nodes:
      | name  | type               | parameters                                        |
      | Start | manualTrigger      |                                                   |
      | Agent | lc.agent           | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatAnthropic | {"model": "bogus-model", "options": {}}           |
    And the node "Model" uses the "anthropicApi" credential "Mock Anthropic"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "model: bogus-model"
