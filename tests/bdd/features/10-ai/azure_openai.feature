@spec-6.8 @phase-5
Feature: Azure OpenAI chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatAzureOpenAi` plugs into the same
  `ai_languageModel` abstraction as the OpenAI Chat Model (see
  `agents.feature`) and speaks the same OpenAI chat-completions format,
  but against an Azure deployment, the way LangChain's `AzureChatOpenAI`
  does: `POST {endpoint}/openai/deployments/{deployment}/chat/completions
  ?api-version={apiVersion}` with an `api-key` header instead of bearer
  auth. The node's `model` parameter is the deployment name. The
  deployment here is a mock server.

  Background:
    Given a mock HTTP service
    And the credential "Mock Azure" of type "azureOpenAiApi" with the data:
      """
      {"apiKey": "azure-key-123", "resourceName": "unused", "apiVersion": "2024-10-21", "endpoint": "%{MOCK_URL}"}
      """

  Scenario: An agent answers with an Azure deployment
    Given a mock Azure OpenAI deployment "gpt4o-prod" that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type                 | parameters                                                                                                                         |
      | Start | manualTrigger        |                                                                                                                                    |
      | Agent | lc.agent             | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatAzureOpenAi | {"model": "gpt4o-prod", "options": {"maxTokens": 256, "temperature": 0.3}}                                                         |
    And the node "Model" uses the "azureOpenAiApi" credential "Mock Azure"
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
    And the last request to "/openai/deployments/gpt4o-prod/chat/completions" had a JSON body matching:
      """
      {
        "model": "gpt4o-prod",
        "max_tokens": 256,
        "temperature": 0.3,
        "messages": [
          {"role": "system", "content": "Answer concisely."},
          {"role": "user", "content": "What is the capital of France?"}
        ]
      }
      """
    And the last request to "/openai/deployments/gpt4o-prod/chat/completions" had the header "api-key" equal to "azure-key-123"
    And the last request to "/openai/deployments/gpt4o-prod/chat/completions" had the query parameter "api-version" equal to "2024-10-21"
    And the node "Model" has run data on the "ai_languageModel" connection
    And the node "Model" recorded a token usage of 15 in total

  Scenario: A Basic LLM Chain returns the deployment's answer
    Given a mock Azure OpenAI deployment "gpt4o-prod" that replies in order:
      """
      [{"role": "assistant", "content": "Paris"}]
      """
    And a workflow with nodes:
      | name  | type                 | parameters                                                                       |
      | Start | manualTrigger        |                                                                                  |
      | Chain | lc.chainLlm          | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatAzureOpenAi | {"model": "gpt4o-prod", "options": {}}                                           |
    And the node "Model" uses the "azureOpenAiApi" credential "Mock Azure"
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
    And azure chat request 1 to "gpt4o-prod" contains a "user" message containing "capital of France"

  Scenario: An agent calls a tool through Azure OpenAI and answers with its result
    Given a mock Azure OpenAI deployment "gpt4o-prod" that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Calculator", "arguments": "{\"input\": \"6 * 7\"}"}}]},
        {"role": "assistant", "content": "The answer is 42."}
      ]
      """
    And a workflow with nodes:
      | name       | type                 | parameters                                                     |
      | Start      | manualTrigger        |                                                                |
      | Agent      | lc.agent             | {"promptType": "define", "text": "={{ $json.question }}", "options": {}} |
      | Model      | lc.lmChatAzureOpenAi | {"model": "gpt4o-prod", "options": {}}                         |
      | Calculator | lc.toolCalculator    |                                                                |
    And the node "Model" uses the "azureOpenAiApi" credential "Mock Azure"
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
    And the mock Azure OpenAI deployment "gpt4o-prod" received 2 chat requests
    And azure chat request 1 to "gpt4o-prod" offers the tool "Calculator"
    And azure chat request 2 to "gpt4o-prod" contains a "tool" message containing "42"
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: A provider error surfaces on the agent without leaking the API key
    Given the mock service responds to POST "/openai/deployments/gpt4o-prod/chat/completions" with status 401 and body:
      """
      {"error": {"code": "401", "message": "Access denied due to invalid subscription key or wrong API endpoint."}}
      """
    And a workflow with nodes:
      | name  | type                 | parameters                                            |
      | Start | manualTrigger        |                                                       |
      | Agent | lc.agent             | {"promptType": "define", "text": "hi", "options": {}} |
      | Model | lc.lmChatAzureOpenAi | {"model": "gpt4o-prod", "options": {}}                |
    And the node "Model" uses the "azureOpenAiApi" credential "Mock Azure"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Access denied"
    And the execution data does not contain "azure-key-123"
