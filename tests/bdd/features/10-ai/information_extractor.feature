@spec-6.8 @phase-5
Feature: Information Extractor
  `@n8n/n8n-nodes-langchain.informationExtractor` asks the connected chat
  model to pull structured data out of each item's text, described by
  attributes (name, type, description, required) or by a JSON example or
  JSON Schema, using LangChain's structured-output format instructions.
  Each item becomes `{"output": <extracted object>}`. An answer that
  doesn't fit is sent back to the model once to be fixed.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: Attributes describe what to extract
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "```json\n{\"name\": \"Ann\", \"age\": 31, \"member\": true, \"since\": \"2020-05-01\", \"extra\": 1}\n```"}]
      """
    And a workflow with nodes:
      | name    | type                    | parameters                                                                       |
      | Start   | manualTrigger           |                                                                                  |
      | Extract | lc.informationExtractor |                                                                                  |
      | Model   | lc.lmChatOpenAi         | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Extract" has parameters:
      """
      {"text": "={{ $json.text }}", "attributes": {"attributes": [
        {"name": "name", "type": "string", "description": "The person's name", "required": true},
        {"name": "age", "type": "number", "description": "Age in years"},
        {"name": "member", "type": "boolean", "description": "Whether they are a member"},
        {"name": "since", "type": "date", "description": "Member since"}
      ]}, "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Extract
      Model -[ai_languageModel]-> Extract
      """
    And the trigger outputs the items:
      """
      [{"text": "Ann (31) has been a member since May 2020"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Extract" outputs:
      """
      [{"output": {"name": "Ann", "age": 31, "member": true, "since": "2020-05-01"}}]
      """
    And chat request 1 has a "user" message with the content "Ann (31) has been a member since May 2020"
    And chat request 1 has a "system" message with the text:
      """
      "You are an expert extraction algorithm.\nOnly extract relevant information from the text.\nIf you do not know the value of an attribute asked to extract, you may omit the attribute's value.\nYou must format your output as a JSON value that adheres to a given \"JSON Schema\" instance.\n\n\"JSON Schema\" is a declarative language that allows you to annotate and validate JSON documents.\n\nFor example, the example \"JSON Schema\" instance {{\"properties\": {{\"foo\": {{\"description\": \"a list of test words\", \"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}}}, \"required\": [\"foo\"]}}}}\nwould match an object with one required property, \"foo\". The \"type\" property specifies \"foo\" must be an \"array\", and the \"description\" property semantically describes it as \"a list of test words\". The items within \"foo\" must be strings.\nThus, the object {{\"foo\": [\"bar\", \"baz\"]}} is a well-formatted instance of this example \"JSON Schema\". The object {{\"properties\": {{\"foo\": [\"bar\", \"baz\"]}}}} is not well-formatted.\n\nYour output will be parsed and type-checked according to the provided schema instance, so make sure all fields in your output match the schema exactly and there are no trailing commas!\n\nHere is the JSON Schema instance your output must adhere to. Include the enclosing markdown codeblock:\n```json\n{\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\",\"description\":\"The person's name\"},\"age\":{\"type\":\"number\",\"description\":\"Age in years\"},\"member\":{\"type\":\"boolean\",\"description\":\"Whether they are a member\"},\"since\":{\"type\":\"string\",\"format\":\"date\",\"description\":\"Member since\"}},\"required\":[\"name\"],\"additionalProperties\":false,\"$schema\":\"http://json-schema.org/draft-07/schema#\"}\n```\n"
      """

  Scenario: A JSON example describes what to extract
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "{\"city\": \"Paris\", \"tags\": [\"food\"], \"address\": {\"zip\": \"75001\"}}"}]
      """
    And a workflow with nodes:
      | name    | type                    | parameters                                                                       |
      | Start   | manualTrigger           |                                                                                  |
      | Extract | lc.informationExtractor |                                                                                  |
      | Model   | lc.lmChatOpenAi         | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Extract" has parameters:
      """
      {"text": "={{ $json.text }}", "schemaType": "fromJson", "jsonSchemaExample": "{\"city\": \"Berlin\", \"tags\": [\"x\"], \"address\": {\"zip\": \"10115\"}}", "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Extract
      Model -[ai_languageModel]-> Extract
      """
    And the trigger outputs the items:
      """
      [{"text": "Great food in Paris 75001"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Extract" outputs:
      """
      [{"output": {"city": "Paris", "tags": ["food"], "address": {"zip": "75001"}}}]
      """
    And chat request 1 has a "system" message with the text:
      """
      "You are an expert extraction algorithm.\nOnly extract relevant information from the text.\nIf you do not know the value of an attribute asked to extract, you may omit the attribute's value.\nYou must format your output as a JSON value that adheres to a given \"JSON Schema\" instance.\n\n\"JSON Schema\" is a declarative language that allows you to annotate and validate JSON documents.\n\nFor example, the example \"JSON Schema\" instance {{\"properties\": {{\"foo\": {{\"description\": \"a list of test words\", \"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}}}, \"required\": [\"foo\"]}}}}\nwould match an object with one required property, \"foo\". The \"type\" property specifies \"foo\" must be an \"array\", and the \"description\" property semantically describes it as \"a list of test words\". The items within \"foo\" must be strings.\nThus, the object {{\"foo\": [\"bar\", \"baz\"]}} is a well-formatted instance of this example \"JSON Schema\". The object {{\"properties\": {{\"foo\": [\"bar\", \"baz\"]}}}} is not well-formatted.\n\nYour output will be parsed and type-checked according to the provided schema instance, so make sure all fields in your output match the schema exactly and there are no trailing commas!\n\nHere is the JSON Schema instance your output must adhere to. Include the enclosing markdown codeblock:\n```json\n{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"},\"tags\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"address\":{\"type\":\"object\",\"properties\":{\"zip\":{\"type\":\"string\"}},\"required\":[\"zip\"],\"additionalProperties\":false}},\"required\":[\"city\",\"tags\",\"address\"],\"additionalProperties\":false,\"$schema\":\"http://json-schema.org/draft-07/schema#\"}\n```\n"
      """

  Scenario: An answer that doesn't fit is sent back to the model to be fixed
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": "Her name is Ann"},
        {"role": "assistant", "content": "{\"name\": \"Ann\"}"}
      ]
      """
    And a workflow with nodes:
      | name    | type                    | parameters                                                                       |
      | Start   | manualTrigger           |                                                                                  |
      | Extract | lc.informationExtractor |                                                                                  |
      | Model   | lc.lmChatOpenAi         | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Extract" has parameters:
      """
      {"text": "={{ $json.text }}", "attributes": {"attributes": [{"name": "name", "type": "string", "description": "The name", "required": true}]}, "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Extract
      Model -[ai_languageModel]-> Extract
      """
    And the trigger outputs the items:
      """
      [{"text": "Ann says hello"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Extract" outputs:
      """
      [{"output": {"name": "Ann"}}]
      """
    And the mock OpenAI API received 2 chat requests
    And chat request 2 contains a "user" message containing "Completion:\n--------------\nHer name is Ann"

  Scenario: When the fix doesn't fit either, the node fails with n8n's parser error
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Her name is Ann"}]
      """
    And a workflow with nodes:
      | name    | type                    | parameters                                                                       |
      | Start   | manualTrigger           |                                                                                  |
      | Extract | lc.informationExtractor |                                                                                  |
      | Model   | lc.lmChatOpenAi         | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Extract" has parameters:
      """
      {"text": "={{ $json.text }}", "attributes": {"attributes": [{"name": "name", "type": "string", "description": "The name", "required": true}]}, "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Extract
      Model -[ai_languageModel]-> Extract
      """
    And the trigger outputs the items:
      """
      [{"text": "Ann says hello"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Extract" failed with an error containing "Model output doesn't fit required format"
