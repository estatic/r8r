@spec-6.8 @phase-5
Feature: AWS Bedrock chat model sub-node
  `@n8n/n8n-nodes-langchain.lmChatAwsBedrock` plugs into the same
  `ai_languageModel` abstraction as the OpenAI Chat Model (see
  `agents.feature`), talking to Bedrock's Converse API the way LangChain's
  `ChatBedrockConverse` does: `POST {runtime}/model/{modelId}/converse`
  (the model id percent-encoded into one path segment), signed with AWS
  Signature V4 for the `bedrock` service, top-level `system` blocks,
  `inferenceConfig`, and `toolUse`/`toolResult` content blocks. A model
  given as an ARN is called in the ARN's region. The credential's Bedrock
  Runtime endpoint override points at a mock server here.

  Background:
    Given a mock HTTP service
    And the credential "Mock AWS" of type "aws" with the data:
      """
      {"region": "us-east-1", "accessKeyId": "AKIDTEST", "secretAccessKey": "aws-secret-123", "customEndpoints": true, "bedrockRuntimeEndpoint": "%{MOCK_URL}"}
      """

  Scenario: An agent answers with Bedrock, using the Converse shape and SigV4
    Given a mock Bedrock API for the model "anthropic.claude-3-haiku-20240307-v1:0" that replies in order:
      """
      [{"content": [{"text": "Paris"}]}]
      """
    And a workflow with nodes:
      | name  | type                | parameters                                                                                                                         |
      | Start | manualTrigger       |                                                                                                                                    |
      | Agent | lc.agent            | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?", "options": {"systemMessage": "Answer concisely."}} |
      | Model | lc.lmChatAwsBedrock | {"model": "anthropic.claude-3-haiku-20240307-v1:0", "options": {"maxTokensToSample": 512, "temperature": 0.2}}                     |
    And the node "Model" uses the "aws" credential "Mock AWS"
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
    And the last Bedrock request for the model "anthropic.claude-3-haiku-20240307-v1:0" had a JSON body matching:
      """
      {
        "system": [{"text": "Answer concisely."}],
        "messages": [
          {"role": "user", "content": [{"text": "What is the capital of France?"}]}
        ],
        "inferenceConfig": {"maxTokens": 512, "temperature": 0.2}
      }
      """
    And the last request to "/model/anthropic.claude-3-haiku-20240307-v1%3A0/converse" was SigV4-signed by "AKIDTEST" with secret "aws-secret-123" for the region "us-east-1" and the service "bedrock"
    And the node "Model" has run data on the "ai_languageModel" connection

  Scenario: A Basic LLM Chain returns Bedrock's answer
    Given a mock Bedrock API for the model "anthropic.claude-3-haiku-20240307-v1:0" that replies in order:
      """
      [{"content": [{"text": "Paris"}]}]
      """
    And a workflow with nodes:
      | name  | type                | parameters                                                                       |
      | Start | manualTrigger       |                                                                                  |
      | Chain | lc.chainLlm         | {"promptType": "define", "text": "=What is the capital of {{ $json.country }}?"} |
      | Model | lc.lmChatAwsBedrock | {"model": "anthropic.claude-3-haiku-20240307-v1:0", "options": {}}               |
    And the node "Model" uses the "aws" credential "Mock AWS"
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
    And bedrock request 1 for the model "anthropic.claude-3-haiku-20240307-v1:0" has a "text" block containing "capital of France"

  Scenario: An agent calls a tool through Bedrock and answers with its result
    Given a mock Bedrock API for the model "anthropic.claude-3-haiku-20240307-v1:0" that replies in order:
      """
      [
        {"content": [{"toolUse": {"toolUseId": "tooluse_1", "name": "Calculator", "input": {"input": "6 * 7"}}}], "stopReason": "tool_use"},
        {"content": [{"text": "The answer is 42."}]}
      ]
      """
    And a workflow with nodes:
      | name       | type                | parameters                                                                                           |
      | Start      | manualTrigger       |                                                                                                      |
      | Agent      | lc.agent            | {"promptType": "define", "text": "={{ $json.question }}", "options": {"systemMessage": "Be exact."}} |
      | Model      | lc.lmChatAwsBedrock | {"model": "anthropic.claude-3-haiku-20240307-v1:0", "options": {}}                                   |
      | Calculator | lc.toolCalculator   |                                                                                                      |
    And the node "Model" uses the "aws" credential "Mock AWS"
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
    And the mock Bedrock API received 2 requests for the model "anthropic.claude-3-haiku-20240307-v1:0"
    And bedrock request 1 for the model "anthropic.claude-3-haiku-20240307-v1:0" offers the tool "Calculator"
    And bedrock request 2 for the model "anthropic.claude-3-haiku-20240307-v1:0" has a "toolUse" block containing "tooluse_1"
    And bedrock request 2 for the model "anthropic.claude-3-haiku-20240307-v1:0" has a "toolResult" block containing "42"
    And the node "Calculator" has run data on the "ai_tool" connection

  Scenario: A model ARN is called in its own region, with temporary credentials
    Given the credential "Mock AWS session" of type "aws" with the data:
      """
      {"region": "us-east-1", "accessKeyId": "ASIATEST", "secretAccessKey": "aws-secret-456", "temporaryCredentials": true, "sessionToken": "session-token-789", "customEndpoints": true, "bedrockRuntimeEndpoint": "%{MOCK_URL}"}
      """
    And a mock Bedrock API for the model "arn:aws:bedrock:eu-west-1:123456789012:inference-profile/eu.anthropic.claude-3-haiku-20240307-v1:0" that replies in order:
      """
      [{"content": [{"text": "ok"}]}]
      """
    And a workflow with nodes:
      | name  | type                | parameters                                                                                                                       |
      | Start | manualTrigger       |                                                                                                                                  |
      | Chain | lc.chainLlm         | {"promptType": "define", "text": "hi"}                                                                                           |
      | Model | lc.lmChatAwsBedrock | {"model": "arn:aws:bedrock:eu-west-1:123456789012:inference-profile/eu.anthropic.claude-3-haiku-20240307-v1:0", "options": {}} |
    And the node "Model" uses the "aws" credential "Mock AWS session"
    And the connections:
      """
      Start -> Chain
      Model -[ai_languageModel]-> Chain
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/model/arn%3Aaws%3Abedrock%3Aeu-west-1%3A123456789012%3Ainference-profile%2Feu.anthropic.claude-3-haiku-20240307-v1%3A0/converse" was SigV4-signed by "ASIATEST" with secret "aws-secret-456" for the region "eu-west-1" and the service "bedrock"
    And the last request to "/model/arn%3Aaws%3Abedrock%3Aeu-west-1%3A123456789012%3Ainference-profile%2Feu.anthropic.claude-3-haiku-20240307-v1%3A0/converse" had the header "x-amz-security-token" equal to "session-token-789"

  Scenario: A provider error surfaces on the agent without leaking the secret key
    Given the mock service responds to POST "/model/anthropic.claude-3-haiku-20240307-v1%3A0/converse" with status 403 and body:
      """
      {"message": "User is not authorized to perform: bedrock:InvokeModel"}
      """
    And a workflow with nodes:
      | name  | type                | parameters                                                         |
      | Start | manualTrigger       |                                                                    |
      | Agent | lc.agent            | {"promptType": "define", "text": "hi", "options": {}}              |
      | Model | lc.lmChatAwsBedrock | {"model": "anthropic.claude-3-haiku-20240307-v1:0", "options": {}} |
    And the node "Model" uses the "aws" credential "Mock AWS"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "not authorized to perform"
    And the execution data does not contain "aws-secret-123"
