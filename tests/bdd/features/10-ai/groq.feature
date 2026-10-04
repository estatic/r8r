@spec-6.8 @phase-5
Feature: Groq chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatGroq` is OpenAI-compatible
  (`POST /chat/completions`), so it reuses the same wire format as the
  OpenAI Chat Model (see `agents.feature`) with its own credential type
  and base URL (`https://api.groq.com/openai/v1` in production). Its
  "Maximum Number of Tokens" option is named `maxTokensToSample`, unlike
  OpenAI's `maxTokens`, but still maps onto `max_tokens` on the wire.

  Background:
    Given a mock HTTP service
    And the credential "Mock Groq" of type "groqApi" with the data:
      """
      {"apiKey": "gsk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: An agent answers with Groq, mapping maxTokensToSample onto max_tokens
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type         | parameters                                                                                                                 |
      | Start | manualTrigger |                                                                                                                            |
      | Agent | lc.agent     | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatGroq | {"model": "llama-3.3-70b-versatile", "options": {"temperature": 0.2, "maxTokensToSample": 256}}                          |
    And the node "Model" uses the "groqApi" credential "Mock Groq"
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
    And the last request to "/v1/chat/completions" had a JSON body matching:
      """
      {
        "model": "llama-3.3-70b-versatile",
        "temperature": 0.2,
        "max_tokens": 256,
        "messages": [
          {"role": "system", "content": "Answer concisely."},
          {"role": "user", "content": "What is the capital of France?"}
        ]
      }
      """
    And the last request to "/v1/chat/completions" had the header "authorization" equal to "Bearer gsk-test-123"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns Groq's answer
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type          | parameters                                                                       |
      | Start | manualTrigger |                                                                                  |
      | Chain | lc.chainLlm   | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatGroq | {"model": "llama-3.3-70b-versatile", "options": {}}                             |
    And the node "Model" uses the "groqApi" credential "Mock Groq"
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

  Scenario: An agent calls a tool through Groq and answers with its result
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Calculator", "arguments": "{\"input\": \"6 * 7\"}"}}]},
        {"role": "assistant", "content": "The answer is 42."}
      ]
      """
    And a workflow with nodes:
      | name       | type              | parameters                                                                                           |
      | Start      | manualTrigger     |                                                                                                      |
      | Agent      | lc.agent          | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatGroq     | {"model": "llama-3.3-70b-versatile", "options": {}}                                                 |
      | Calculator | lc.toolCalculator |                                                                                                      |
    And the node "Model" uses the "groqApi" credential "Mock Groq"
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
    And chat request 2 contains a "tool" message containing "42"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: A provider error surfaces on the agent without leaking the API key
    Given the mock service responds to POST "/v1/chat/completions" with status 401 and body:
      """
      {"error": {"message": "Invalid API Key", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type          | parameters                                              |
      | Start | manualTrigger |                                                         |
      | Agent | lc.agent      | {"promptType": "define", "text": "hi", "options": {}}  |
      | Model | lc.lmChatGroq | {"model": "llama-3.3-70b-versatile", "options": {}}    |
    And the node "Model" uses the "groqApi" credential "Mock Groq"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Invalid API Key"
    And the execution data does not contain "gsk-test-123"

  Scenario: An unknown model surfaces Groq's error
    Given the mock service responds to POST "/v1/chat/completions" with status 404 and body:
      """
      {"error": {"message": "The model `bogus` does not exist", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type          | parameters                                             |
      | Start | manualTrigger |                                                        |
      | Agent | lc.agent      | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatGroq | {"model": "bogus", "options": {}}                     |
    And the node "Model" uses the "groqApi" credential "Mock Groq"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "does not exist"
