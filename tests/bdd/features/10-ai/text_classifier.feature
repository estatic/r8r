@spec-6.8 @phase-5
Feature: Text Classifier
  `@n8n/n8n-nodes-langchain.textClassifier` asks the connected chat model
  which of its categories apply to each item's text (a boolean per
  category in LangChain's structured output) and passes the item,
  unchanged, to the output of every category that applies: one only, or
  several with "Allow Multiple Classes To Be True". With the "Other"
  fallback an extra last output takes items that fit no category.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: Items go to the output of their category
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "```json\n{\"Billing\": false, \"Support\": true}\n```"}]
      """
    And a workflow with nodes:
      | name     | type              | parameters                                                                       |
      | Start    | manualTrigger     |                                                                                  |
      | Classify | lc.textClassifier |                                                                                  |
      | Model    | lc.lmChatOpenAi   | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Classify" has parameters:
      """
      {"inputText": "={{ $json.text }}", "categories": {"categories": [{"category": "Billing", "description": "Invoices and payments"}, {"category": "Support", "description": "Help with the product"}]}, "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Classify
      Model -[ai_languageModel]-> Classify
      """
    And the trigger outputs the items:
      """
      [{"text": "The app crashes on start"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Classify" is empty
    And output 1 of the node "Classify" is:
      """
      [{"text": "The app crashes on start"}]
      """
    And chat request 1 has a "user" message with the content "The app crashes on start"
    And chat request 1 has a "system" message with the text:
      """
      "Please classify the text provided by the user into one of the following categories: Billing, Support, and use the provided formatting instructions below. Don't explain, and only output the json.\n\tYou must format your output as a JSON value that adheres to a given \"JSON Schema\" instance.\n\n\"JSON Schema\" is a declarative language that allows you to annotate and validate JSON documents.\n\nFor example, the example \"JSON Schema\" instance {{\"properties\": {{\"foo\": {{\"description\": \"a list of test words\", \"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}}}, \"required\": [\"foo\"]}}}}\nwould match an object with one required property, \"foo\". The \"type\" property specifies \"foo\" must be an \"array\", and the \"description\" property semantically describes it as \"a list of test words\". The items within \"foo\" must be strings.\nThus, the object {{\"foo\": [\"bar\", \"baz\"]}} is a well-formatted instance of this example \"JSON Schema\". The object {{\"properties\": {{\"foo\": [\"bar\", \"baz\"]}}}} is not well-formatted.\n\nYour output will be parsed and type-checked according to the provided schema instance, so make sure all fields in your output match the schema exactly and there are no trailing commas!\n\nHere is the JSON Schema instance your output must adhere to. Include the enclosing markdown codeblock:\n```json\n{\"type\":\"object\",\"properties\":{\"Billing\":{\"type\":\"boolean\",\"description\":\"Should be true if the input has category \\\"Billing\\\" (description: Invoices and payments)\"},\"Support\":{\"type\":\"boolean\",\"description\":\"Should be true if the input has category \\\"Support\\\" (description: Help with the product)\"}},\"required\":[\"Billing\",\"Support\"],\"additionalProperties\":false,\"$schema\":\"http://json-schema.org/draft-07/schema#\"}\n```\n\n\tCategories are mutually exclusive, and only one can be true\n\tIf there is not a very fitting category, select none of the categories."
      """

  Scenario: With multiple classes an item goes to every category that applies
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "{\"Billing\": true, \"Support\": true}"}]
      """
    And a workflow with nodes:
      | name     | type              | parameters                                                                       |
      | Start    | manualTrigger     |                                                                                  |
      | Classify | lc.textClassifier |                                                                                  |
      | Model    | lc.lmChatOpenAi   | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Classify" has parameters:
      """
      {"inputText": "={{ $json.text }}", "categories": {"categories": [{"category": "Billing", "description": "Invoices"}, {"category": "Support", "description": "Help"}]}, "options": {"multiClass": true}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Classify
      Model -[ai_languageModel]-> Classify
      """
    And the trigger outputs the items:
      """
      [{"text": "I was charged twice and the app crashes"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Classify" has 1 item
    And output 1 of the node "Classify" has 1 item
    And chat request 1 contains a "system" message containing "Categories are not mutually exclusive, and multiple can be true"

  Scenario: The Other fallback adds a last output for items that fit nothing
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "{\"Billing\": false, \"Support\": false, \"fallback\": true}"}]
      """
    And a workflow with nodes:
      | name     | type              | parameters                                                                       |
      | Start    | manualTrigger     |                                                                                  |
      | Classify | lc.textClassifier |                                                                                  |
      | Model    | lc.lmChatOpenAi   | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Classify" has parameters:
      """
      {"inputText": "={{ $json.text }}", "categories": {"categories": [{"category": "Billing", "description": "Invoices"}, {"category": "Support", "description": "Help"}]}, "options": {"fallback": "other"}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Classify
      Model -[ai_languageModel]-> Classify
      """
    And the trigger outputs the items:
      """
      [{"text": "What a lovely day"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Classify" is empty
    And output 1 of the node "Classify" is empty
    And output 2 of the node "Classify" is:
      """
      [{"text": "What a lovely day"}]
      """
    And chat request 1 contains a "system" message containing "If no categories apply, select the \"fallback\" option."
    And chat request 1 contains a "system" message containing "\"fallback\":{\"type\":\"boolean\",\"description\":\"Should be true if none of the other categories apply\"}"

  Scenario: An answer that doesn't fit the schema fails with n8n's parser error
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Support, probably"}]
      """
    And a workflow with nodes:
      | name     | type              | parameters                                                                       |
      | Start    | manualTrigger     |                                                                                  |
      | Classify | lc.textClassifier |                                                                                  |
      | Model    | lc.lmChatOpenAi   | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Classify" has parameters:
      """
      {"inputText": "={{ $json.text }}", "categories": {"categories": [{"category": "Billing", "description": "Invoices"}, {"category": "Support", "description": "Help"}]}, "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Classify
      Model -[ai_languageModel]-> Classify
      """
    And the trigger outputs the items:
      """
      [{"text": "The app crashes"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Classify" failed with an error containing "Model output doesn't fit required format"
