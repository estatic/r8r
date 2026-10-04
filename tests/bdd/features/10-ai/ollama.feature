@spec-6.8 @phase-5
Feature: Ollama chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatOllama` plugs into the same
  `ai_languageModel` abstraction as the OpenAI Chat Model (see
  `agents.feature`), talking to Ollama's native `/api/chat` instead of an
  OpenAI-compatible endpoint. The model here is a mock Ollama server.

  Background:
    Given a mock HTTP service
    And the credential "Mock Ollama" of type "ollamaApi" with the data:
      """
      {"baseUrl": "%{MOCK_URL}"}
      """

  Scenario: An agent answers with Ollama, mapping options onto the native request
    Given a mock Ollama API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                                                                                       |
      | Start | manualTrigger   |                                                                                                                                                  |
      | Agent | lc.agent        | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}}            |
      | Model | lc.lmChatOllama | {"model": "llama3.2", "options": {"temperature": 0.2, "topK": 40, "topP": 0.9, "numCtx": 4096, "keepAlive": "10m", "format": "json"}}           |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama"
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
    And the last request to "/api/chat" had a JSON body matching:
      """
      {
        "model": "llama3.2",
        "stream": false,
        "format": "json",
        "keep_alive": "10m",
        "options": {"temperature": 0.2, "top_k": 40, "top_p": 0.9, "num_ctx": 4096},
        "messages": [
          {"role": "system", "content": "Answer concisely."},
          {"role": "user", "content": "What is the capital of France?"}
        ]
      }
      """
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns Ollama's answer
    Given a mock Ollama API that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                       |
      | Start | manualTrigger   |                                                                                  |
      | Chain | lc.chainLlm     | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatOllama | {"model": "llama3.2", "options": {}}                                            |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama"
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
    And ollama chat request 1 contains a "user" message containing "capital of France"

  Scenario: An agent calls a tool through Ollama and answers with its result
    Given a mock Ollama API that replies in order:
      """
      [
        {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "Calculator", "arguments": {"input": "6 * 7"}}}]},
        {"role": "assistant", "content": "The answer is 42."}
      ]
      """
    And a workflow with nodes:
      | name       | type              | parameters                                                                                           |
      | Start      | manualTrigger     |                                                                                                      |
      | Agent      | lc.agent          | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatOllama   | {"model": "llama3.2", "options": {}}                                                                 |
      | Calculator | lc.toolCalculator |                                                                                                      |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama"
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
    And the mock Ollama API received 2 chat requests
    And ollama chat request 1 offers the tool "Calculator"
    And ollama chat request 2 contains a "tool" message containing "42"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: An unreachable Ollama server fails the node
    Given the credential "Unreachable Ollama" of type "ollamaApi" with the data:
      """
      {"baseUrl": "http://127.0.0.1:18832"}
      """
    And a workflow with nodes:
      | name  | type            | parameters                                             |
      | Start | manualTrigger   |                                                        |
      | Agent | lc.agent        | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatOllama | {"model": "llama3.2", "options": {}}                  |
    And the node "Model" uses the "ollamaApi" credential "Unreachable Ollama"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Ollama could not be reached"

  Scenario: An unknown model surfaces Ollama's error
    Given the mock service responds to POST "/api/chat" with status 404 and body:
      """
      {"error": "model \"bogus\" not found, try pulling it first"}
      """
    And a workflow with nodes:
      | name  | type            | parameters                                              |
      | Start | manualTrigger   |                                                         |
      | Agent | lc.agent        | {"promptType": "define", "text": "hi", "options": {}}  |
      | Model | lc.lmChatOllama | {"model": "bogus", "options": {}}                      |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "not found"

  Scenario: An API key is sent as a bearer header, and never leaks into execution data
    Given the credential "Mock Ollama With Key" of type "ollamaApi" with the data:
      """
      {"baseUrl": "%{MOCK_URL}", "apiKey": "ollama-secret-1"}
      """
    And a mock Ollama API that replies in order:
      """
      [{"role": "assistant", "content": "hi there"}]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                              |
      | Start | manualTrigger   |                                                         |
      | Agent | lc.agent        | {"promptType": "define", "text": "hi", "options": {}}  |
      | Model | lc.lmChatOllama | {"model": "llama3.2", "options": {}}                   |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama With Key"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat" had the header "authorization" equal to "Bearer ollama-secret-1"
    And the execution data does not contain "ollama-secret-1"

  Scenario: No Authorization header is sent when no API key is configured
    Given a mock Ollama API that replies in order:
      """
      [{"role": "assistant", "content": "hi there"}]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                              |
      | Start | manualTrigger   |                                                         |
      | Agent | lc.agent        | {"promptType": "define", "text": "hi", "options": {}}  |
      | Model | lc.lmChatOllama | {"model": "llama3.2", "options": {}}                   |
    And the node "Model" uses the "ollamaApi" credential "Mock Ollama"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat" had the header "authorization" equal to "<absent>"
