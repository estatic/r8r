@spec-6.8 @phase-5
Feature: AI Agent and LLM chain nodes
  AI root nodes get their model, memory and tools from sub-nodes over
  `ai_*` connections. Only observable behaviour is matched: outputs, the
  requests sent to the provider, and sub-runs recorded in runData with
  token usage. The model here is a mock OpenAI-compatible API.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: A basic LLM chain returns the model's answer
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name   | type            | parameters                                                                         |
      | Start  | manualTrigger   |                                                                                    |
      | Chain  | lc.chainLlm     | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"}   |
      | Model  | lc.lmChatOpenAi | {"model": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"}, "options": {}}   |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
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
    And chat request 1 contains a "user" message containing "capital of France"
    And the last request to "/v1/chat/completions" had the header "authorization" equal to "Bearer sk-test-123"

  Scenario: An agent calls a tool and answers with its result
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Calculator", "arguments": "{\"input\": \"6 * 7\"}"}}]},
        {"role": "assistant", "content": "The answer is 42."}
      ]
      """
    And a workflow with nodes:
      | name       | type              | parameters                                                                                         |
      | Start      | manualTrigger     |                                                                                                    |
      | Agent      | lc.agent          | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatOpenAi   | {"model": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"}, "options": {}}                   |
      | Calculator | lc.toolCalculator |                                                                                                    |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
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
    And the mock OpenAI API received 2 chat requests
    And chat request 1 offers the tool "Calculator"
    And chat request 1 contains a "system" message containing "Be exact."
    And chat request 2 contains a "tool" message containing "42"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Calculator" has run data on the "ai_tool" connection
    And the node "Model" recorded a token usage of 30 in total

  Scenario: An agent gives up after its maximum number of iterations
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": null, "tool_calls": [{"id": "call_n", "type": "function", "function": {"name": "Calculator", "arguments": "{\"input\": \"1 + 1\"}"}}]}]
      """
    And a workflow with nodes:
      | name       | type              | parameters                                                                   |
      | Start      | manualTrigger     |                                                                              |
      | Agent      | lc.agent          | {"promptType": "define", "text": "loop forever", "options": {"maxIterations": 3}} |
      | Model      | lc.lmChatOpenAi   | {"model": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"}, "options": {}} |
      | Calculator | lc.toolCalculator |                                                                              |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Calculator -[ai_tool]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the field "output" of item 0 from the node "Agent" is "$contains:max iterations"
    And the mock OpenAI API received 3 chat requests

  Scenario: Window buffer memory carries the conversation between runs
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Nice to meet you, Ada."}, {"role": "assistant", "content": "Your name is Ada."}]
      """
    And a workflow with nodes:
      | name   | type                  | parameters                                                                         |
      | Start  | manualTrigger         |                                                                                    |
      | Agent  | lc.agent              | {"promptType": "define", "text": "={{ $json.message }}", "options": {}}            |
      | Model  | lc.lmChatOpenAi       | {"model": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"}, "options": {}}   |
      | Memory | lc.memoryBufferWindow | {"sessionIdType": "customKey", "sessionKey": "session-1", "contextWindowLength": 5} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Memory -[ai_memory]-> Agent
      """
    And the trigger outputs the items:
      """
      [{"message": "My name is Ada."}, {"message": "What is my name?"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 2 contains a "user" message containing "My name is Ada."
    And chat request 2 contains a "assistant" message containing "Nice to meet you, Ada."

  Scenario: A provider error surfaces on the agent without leaking the API key
    Given the mock service responds to POST "/v1/chat/completions" with status 401 and body:
      """
      {"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                       |
      | Start | manualTrigger   |                                                                                  |
      | Agent | lc.agent        | {"promptType": "define", "text": "hi", "options": {}}                            |
      | Model | lc.lmChatOpenAi | {"model": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Incorrect API key provided"
    And the execution data does not contain "sk-test-123"
