@spec-6.8 @phase-5
Feature: Sentiment Analysis
  `@n8n/n8n-nodes-langchain.sentimentAnalysis` asks the connected chat
  model to classify each item's text into one of its categories (default
  "Positive, Neutral, Negative"), with LangChain's structured-output
  format instructions in the system prompt, and routes the item to the
  output of that category with `sentimentAnalysis.category` added
  (plus strength and confidence with "Include Detailed Results").

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: Items are routed to the output of their sentiment
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "```json\n{\"sentiment\": \"Positive\", \"strength\": 0.9, \"confidence\": 0.95}\n```"}]
      """
    And a workflow with nodes:
      | name      | type                 | parameters                                                                       |
      | Start     | manualTrigger        |                                                                                  |
      | Sentiment | lc.sentimentAnalysis | {"inputText": "={{ $json.review }}", "options": {"includeDetailedResults": true}} |
      | Model     | lc.lmChatOpenAi      | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Sentiment
      Model -[ai_languageModel]-> Sentiment
      """
    And the trigger outputs the items:
      """
      [{"review": "I love it"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Sentiment" is:
      """
      [{"review": "I love it", "sentimentAnalysis": {"category": "Positive", "strength": 0.9, "confidence": 0.95}}]
      """
    And output 1 of the node "Sentiment" is empty
    And output 2 of the node "Sentiment" is empty
    And chat request 1 has a "user" message with the content "I love it"
    And chat request 1 has a "system" message with the text:
      """
      "You are highly intelligent and accurate sentiment analyzer. Analyze the sentiment of the provided text. Categorize it into one of the following: Positive, Neutral, Negative. Use the provided formatting instructions. Only output the JSON.\n\t\t\t\tYou must format your output as a JSON value that adheres to a given \"JSON Schema\" instance.\n\n\"JSON Schema\" is a declarative language that allows you to annotate and validate JSON documents.\n\nFor example, the example \"JSON Schema\" instance {{\"properties\": {{\"foo\": {{\"description\": \"a list of test words\", \"type\": \"array\", \"items\": {{\"type\": \"string\"}}}}}}, \"required\": [\"foo\"]}}}}\nwould match an object with one required property, \"foo\". The \"type\" property specifies \"foo\" must be an \"array\", and the \"description\" property semantically describes it as \"a list of test words\". The items within \"foo\" must be strings.\nThus, the object {{\"foo\": [\"bar\", \"baz\"]}} is a well-formatted instance of this example \"JSON Schema\". The object {{\"properties\": {{\"foo\": [\"bar\", \"baz\"]}}}} is not well-formatted.\n\nYour output will be parsed and type-checked according to the provided schema instance, so make sure all fields in your output match the schema exactly and there are no trailing commas!\n\nHere is the JSON Schema instance your output must adhere to. Include the enclosing markdown codeblock:\n```json\n{\"type\":\"object\",\"properties\":{\"sentiment\":{\"type\":\"string\",\"enum\":[\"Positive\",\"Neutral\",\"Negative\"]},\"strength\":{\"type\":\"number\",\"minimum\":0,\"maximum\":1,\"description\":\"Strength score for sentiment in relation to the category\"},\"confidence\":{\"type\":\"number\",\"minimum\":0,\"maximum\":1}},\"required\":[\"sentiment\",\"strength\",\"confidence\"],\"additionalProperties\":false,\"$schema\":\"http://json-schema.org/draft-07/schema#\"}\n```\n"
      """

  Scenario: Custom categories make the outputs
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "{\"sentiment\": \"Angry\", \"strength\": 0.8, \"confidence\": 0.9}"}]
      """
    And a workflow with nodes:
      | name      | type                 | parameters                                                                       |
      | Start     | manualTrigger        |                                                                                  |
      | Sentiment | lc.sentimentAnalysis | {"inputText": "={{ $json.review }}", "options": {"categories": "Happy, Angry"}}  |
      | Model     | lc.lmChatOpenAi      | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Sentiment
      Model -[ai_languageModel]-> Sentiment
      """
    And the trigger outputs the items:
      """
      [{"review": "This is terrible"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Sentiment" is empty
    And output 1 of the node "Sentiment" is:
      """
      [{"review": "This is terrible", "sentimentAnalysis": {"category": "Angry"}}]
      """

  Scenario: An answer that doesn't fit the schema fails the node
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "I think it is positive"}]
      """
    And a workflow with nodes:
      | name      | type                 | parameters                                                                       |
      | Start     | manualTrigger        |                                                                                  |
      | Sentiment | lc.sentimentAnalysis | {"inputText": "={{ $json.review }}", "options": {}}                              |
      | Model     | lc.lmChatOpenAi      | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Sentiment
      Model -[ai_languageModel]-> Sentiment
      """
    And the trigger outputs the items:
      """
      [{"review": "I love it"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Sentiment" failed with an error containing "Error during parsing of LLM output, please check your LLM model and configuration"

  Scenario: With auto-fixing, the model is asked once more to fix its answer
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": "I think it is positive"},
        {"role": "assistant", "content": "{\"sentiment\": \"Positive\", \"strength\": 0.7, \"confidence\": 0.8}"}
      ]
      """
    And a workflow with nodes:
      | name      | type                 | parameters                                                                       |
      | Start     | manualTrigger        |                                                                                  |
      | Sentiment | lc.sentimentAnalysis | {"inputText": "={{ $json.review }}", "options": {"enableAutoFixing": true}}      |
      | Model     | lc.lmChatOpenAi      | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Sentiment
      Model -[ai_languageModel]-> Sentiment
      """
    And the trigger outputs the items:
      """
      [{"review": "I love it"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And output 0 of the node "Sentiment" is:
      """
      [{"review": "I love it", "sentimentAnalysis": {"category": "Positive"}}]
      """
    And the mock OpenAI API received 2 chat requests
    And chat request 2 contains a "user" message containing "Completion:\n--------------\nI think it is positive"
    And chat request 2 contains a "user" message containing "Please try again. Please only respond with an answer that satisfies the constraints laid out in the Instructions:"
