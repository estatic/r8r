@spec-6.8 @phase-5
Feature: Google Gemini chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatGoogleGemini` plugs into the same
  `ai_languageModel` abstraction as the OpenAI Chat Model (see
  `agents.feature`), talking to Google's Generative Language API the way
  LangChain's `ChatGoogleGenerativeAI` does:
  `POST {host}/v1beta/models/{model}:generateContent` with an
  `x-goog-api-key` header, `contents` with `user`/`model` roles and
  `parts`, a top-level `systemInstruction`, `generationConfig`, and
  `functionCall`/`functionResponse` parts for tool calls. The model name
  may carry the `models/` prefix the editor's model list uses. The model
  here is a mock Gemini server.

  Background:
    Given a mock HTTP service
    And the credential "Mock Gemini" of type "googlePalmApi" with the data:
      """
      {"apiKey": "AIza-test-123", "host": "%{MOCK_URL}"}
      """

  Scenario: An agent answers with Gemini, using the generateContent shape
    Given a mock Gemini API for the model "gemini-2.5-flash" that replies in order:
      """
      [{"parts": [{"text": "Paris"}]}]
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                                                                                                         |
      | Start | manualTrigger         |                                                                                                                                    |
      | Agent | lc.agent              | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatGoogleGemini | {"modelName": "models/gemini-2.5-flash", "options": {"maxOutputTokens": 512, "temperature": 0.2, "topK": 20, "topP": 0.9}}        |
    And the node "Model" uses the "googlePalmApi" credential "Mock Gemini"
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
    And the last request to "/v1beta/models/gemini-2.5-flash:generateContent" had a JSON body matching:
      """
      {
        "contents": [
          {"role": "user", "parts": [{"text": "What is the capital of France?"}]}
        ],
        "systemInstruction": {"role": "system", "parts": [{"text": "Answer concisely."}]},
        "generationConfig": {"maxOutputTokens": 512, "temperature": 0.2, "topK": 20, "topP": 0.9}
      }
      """
    And the last request to "/v1beta/models/gemini-2.5-flash:generateContent" had the header "x-goog-api-key" equal to "AIza-test-123"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns Gemini's answer, model named without the prefix
    Given a mock Gemini API for the model "gemini-2.0-flash" that replies in order:
      """
      [{"parts": [{"text": "Paris"}]}]
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                                                       |
      | Start | manualTrigger         |                                                                                  |
      | Chain | lc.chainLlm           | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatGoogleGemini | {"modelName": "gemini-2.0-flash", "options": {}}                                 |
    And the node "Model" uses the "googlePalmApi" credential "Mock Gemini"
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
    And gemini request 1 for the model "gemini-2.0-flash" has a "text" part containing "capital of France"

  Scenario: An agent calls a tool through Gemini and answers with its result
    Given a mock Gemini API for the model "gemini-2.5-flash" that replies in order:
      """
      [
        {"parts": [{"functionCall": {"name": "Calculator", "args": {"input": "6 * 7"}}}]},
        {"parts": [{"text": "The answer is 42."}]}
      ]
      """
    And a workflow with nodes:
      | name       | type                  | parameters                                                                                           |
      | Start      | manualTrigger         |                                                                                                      |
      | Agent      | lc.agent              | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatGoogleGemini | {"modelName": "models/gemini-2.5-flash", "options": {}}                                              |
      | Calculator | lc.toolCalculator     |                                                                                                      |
    And the node "Model" uses the "googlePalmApi" credential "Mock Gemini"
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
    And the mock Gemini API received 2 requests for the model "gemini-2.5-flash"
    And gemini request 1 for the model "gemini-2.5-flash" offers the tool "Calculator"
    And gemini request 2 for the model "gemini-2.5-flash" has a "functionCall" part containing "Calculator"
    And gemini request 2 for the model "gemini-2.5-flash" has a "functionResponse" part containing "42"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: Safety settings are sent with the request
    Given a mock Gemini API for the model "gemini-2.5-flash" that replies in order:
      """
      [{"parts": [{"text": "ok"}]}]
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                                                                                                                                                  |
      | Start | manualTrigger         |                                                                                                                                                                             |
      | Chain | lc.chainLlm           | {"promptType": "define", "text": "hi"}                                                                                                                                      |
      | Model | lc.lmChatGoogleGemini | {"modelName": "models/gemini-2.5-flash", "options": {"safetySettings": {"values": [{"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_ONLY_HIGH"}]}}} |
    And the node "Model" uses the "googlePalmApi" credential "Mock Gemini"
    And the connections:
      """
      Start -> Chain
      Model -[ai_languageModel]-> Chain
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1beta/models/gemini-2.5-flash:generateContent" had a JSON body matching:
      """
      {"safetySettings": [{"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_ONLY_HIGH"}]}
      """

  Scenario: A provider error surfaces on the agent without leaking the API key
    Given the mock service responds to POST "/v1beta/models/gemini-2.5-flash:generateContent" with status 400 and body:
      """
      {"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}
      """
    And a workflow with nodes:
      | name  | type                  | parameters                                              |
      | Start | manualTrigger         |                                                         |
      | Agent | lc.agent              | {"promptType": "define", "text": "hi", "options": {}}   |
      | Model | lc.lmChatGoogleGemini | {"modelName": "models/gemini-2.5-flash", "options": {}} |
    And the node "Model" uses the "googlePalmApi" credential "Mock Gemini"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "API key not valid"
    And the execution data does not contain "AIza-test-123"
