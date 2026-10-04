@spec-6.8 @phase-5
Feature: Mistral Cloud chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatMistralCloud` is OpenAI-compatible
  (`POST /chat/completions`), so it reuses the same wire format as the
  OpenAI Chat Model (see `agents.feature`) with its own credential type
  and base URL (`https://api.mistral.ai/v1` in production). Its two
  provider-specific options, "Enable Safe Mode" and "Random Seed", map
  onto Mistral's `safe_prompt`/`random_seed` request fields.

  Background:
    Given a mock HTTP service
    And the credential "Mock Mistral" of type "mistralCloudApi" with the data:
      """
      {"apiKey": "mistral-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: An agent answers with Mistral, mapping safeMode and randomSeed onto the wire
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type                   | parameters                                                                                                                            |
      | Start | manualTrigger          |                                                                                                                                       |
      | Agent | lc.agent               | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatMistralCloud  | {"model": "mistral-large-latest", "options": {"temperature": 0.2, "safeMode": true, "randomSeed": 42}}                             |
    And the node "Model" uses the "mistralCloudApi" credential "Mock Mistral"
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
        "model": "mistral-large-latest",
        "temperature": 0.2,
        "safe_prompt": true,
        "random_seed": 42,
        "messages": [
          {"role": "system", "content": "Answer concisely."},
          {"role": "user", "content": "What is the capital of France?"}
        ]
      }
      """
    And the last request to "/v1/chat/completions" had the header "authorization" equal to "Bearer mistral-test-123"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns Mistral's answer
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                                                       |
      | Start | manualTrigger         |                                                                                  |
      | Chain | lc.chainLlm           | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatMistralCloud | {"model": "mistral-small-latest", "options": {}}                               |
    And the node "Model" uses the "mistralCloudApi" credential "Mock Mistral"
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

  Scenario: An agent calls a tool through Mistral and answers with its result
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Calculator", "arguments": "{\"input\": \"6 * 7\"}"}}]},
        {"role": "assistant", "content": "The answer is 42."}
      ]
      """
    And a workflow with nodes:
      | name       | type                  | parameters                                                                                           |
      | Start      | manualTrigger         |                                                                                                      |
      | Agent      | lc.agent              | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatMistralCloud | {"model": "mistral-large-latest", "options": {}}                                                    |
      | Calculator | lc.toolCalculator     |                                                                                                      |
    And the node "Model" uses the "mistralCloudApi" credential "Mock Mistral"
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
      {"error": {"message": "Unauthorized", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                        |
      | Start | manualTrigger         |                                                   |
      | Agent | lc.agent              | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatMistralCloud | {"model": "mistral-large-latest", "options": {}}  |
    And the node "Model" uses the "mistralCloudApi" credential "Mock Mistral"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Unauthorized"
    And the execution data does not contain "mistral-test-123"

  Scenario: A rate limit surfaces on the agent
    Given the mock service responds to POST "/v1/chat/completions" with status 429 and body:
      """
      {"error": {"message": "Requests rate limit exceeded", "type": "rate_limit_error"}}
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                         |
      | Start | manualTrigger         |                                                    |
      | Agent | lc.agent              | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatMistralCloud | {"model": "mistral-large-latest", "options": {}}   |
    And the node "Model" uses the "mistralCloudApi" credential "Mock Mistral"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Requests rate limit exceeded"
