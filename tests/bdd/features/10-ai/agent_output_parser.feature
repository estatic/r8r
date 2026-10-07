@spec-6.8 @phase-5
Feature: AI Agent structured output
  With "Require Specific Output Format" (`hasOutputParser`) and a
  Structured Output Parser connected over `ai_outputParser`, the agent
  offers the model an extra tool, `format_final_json_response`, whose
  schema is the parser's (`{"output": <schema>}`), and tells it to answer
  through that tool. The agent's item is then `{"output": <parsed object>}`.
  A plain-text JSON answer is parsed the same way. Without a system
  message option the agent sends none (from v1.9), only the formatting
  instructions when a parser is connected.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: The model answers through format_final_json_response
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "format_final_json_response", "arguments": "{\"output\": {\"city\": \"Paris\", \"temp\": 21}}"}}]}]
      """
    And a workflow with nodes:
      | name   | type                      | parameters                                                                                                  |
      | Start  | manualTrigger             |                                                                                                             |
      | Agent  | lc.agent                  | {"promptType": "define", "text": "Weather in Paris?", "hasOutputParser": true, "options": {}}               |
      | Model  | lc.lmChatOpenAi           | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                            |
      | Parser | lc.outputParserStructured |                                                                                                             |
    And the node "Parser" has parameters:
      """
      {"jsonSchemaExample": "{\"city\": \"Berlin\", \"temp\": 5}"}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Parser -[ai_outputParser]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": {"city": "Paris", "temp": 21}}]
      """
    And the mock OpenAI API received 1 chat request
    And chat request 1 offers exactly the tool "format_final_json_response":
      """
      {"type":"function","function":{"name":"format_final_json_response","description":"Use this tool to format your final response to the user in a structured JSON format. This tool validates your output against a schema to ensure it meets the required format. ONLY use this tool when you have completed all necessary reasoning and are ready to provide your final answer. Do not use this tool for intermediate steps or for asking questions. The output from this tool will be directly returned to the user.","parameters":{"type":"object","properties":{"output":{"type":"object","properties":{"city":{"type":"string"},"temp":{"type":"number"}},"required":["city","temp"],"additionalProperties":false}},"required":["output"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"},"strict":false}}
      """
    And chat request 1 has a "system" message with the text:
      """
      "IMPORTANT: For your response to user, you MUST use the `format_final_json_response` tool with your complete answer formatted according to the required schema. Do not attempt to format the JSON manually - always use this tool. Your response will be rejected if it is not properly formatted through this tool. Only use this tool once you are ready to provide your final answer."
      """
    And the node "Parser" has run data on the "ai_outputParser" connection

  Scenario: Without a system message option the agent sends none
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Hi"}]
      """
    And a workflow with nodes:
      | name  | type            | parameters                                                                       |
      | Start | manualTrigger   |                                                                                  |
      | Agent | lc.agent        | {"promptType": "define", "text": "Hello", "options": {}}                         |
      | Model | lc.lmChatOpenAi | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 has no "system" message

  Scenario: A plain JSON answer is parsed too, after the configured system message
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "```json\n{\"output\": {\"city\": \"Rome\", \"temp\": 25}}\n```"}]
      """
    And a workflow with nodes:
      | name   | type                      | parameters                                                                                                                  |
      | Start  | manualTrigger             |                                                                                                                             |
      | Agent  | lc.agent                  | {"promptType": "define", "text": "Weather in Rome?", "hasOutputParser": true, "options": {"systemMessage": "Be brief."}}     |
      | Model  | lc.lmChatOpenAi           | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                            |
      | Parser | lc.outputParserStructured |                                                                                                                             |
    And the node "Parser" has parameters:
      """
      {"jsonSchemaExample": "{\"city\": \"Berlin\", \"temp\": 5}"}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Parser -[ai_outputParser]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": {"city": "Rome", "temp": 25}}]
      """
    And chat request 1 has a "system" message with the text:
      """
      "Be brief.\n\nIMPORTANT: For your response to user, you MUST use the `format_final_json_response` tool with your complete answer formatted according to the required schema. Do not attempt to format the JSON manually - always use this tool. Your response will be rejected if it is not properly formatted through this tool. Only use this tool once you are ready to provide your final answer."
      """

  Scenario: An answer that doesn't match the schema fails the agent
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "format_final_json_response", "arguments": "{\"output\": {\"city\": \"Paris\"}}"}}]}]
      """
    And a workflow with nodes:
      | name   | type                      | parameters                                                                                    |
      | Start  | manualTrigger             |                                                                                               |
      | Agent  | lc.agent                  | {"promptType": "define", "text": "Weather in Paris?", "hasOutputParser": true, "options": {}} |
      | Model  | lc.lmChatOpenAi           | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}              |
      | Parser | lc.outputParserStructured |                                                                                               |
    And the node "Parser" has parameters:
      """
      {"jsonSchemaExample": "{\"city\": \"Berlin\", \"temp\": 5}"}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Parser -[ai_outputParser]-> Agent
      """
    When I execute the workflow
    Then the execution fails
    And the node "Agent" failed with an error containing "Model output doesn't fit required format"
