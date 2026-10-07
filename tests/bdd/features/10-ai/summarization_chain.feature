@spec-6.8 @phase-5
Feature: Summarization Chain
  `@n8n/n8n-nodes-langchain.chainSummarization` (v2) turns each item's JSON
  into documents (its string values), splits them into chunks
  (`RecursiveCharacterTextSplitter`, default 1000/200) and summarizes them
  with LangChain's summarization chains: map-reduce (default; chunks that
  fit one prompt are summarized in one call, otherwise each is summarized
  first), stuff (all chunks in one prompt) or refine (one chunk at a time,
  improving the previous summary). Each prompt goes to the chat model as a
  single user message; the item becomes `{"output": {...}}`.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  Scenario: Short text is summarized in one call with LangChain's default prompt
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Cats are good pets."}]
      """
    And a workflow with nodes:
      | name      | type                  | parameters                                                                       |
      | Start     | manualTrigger         |                                                                                  |
      | Summarize | lc.chainSummarization | {"options": {}}                                                                  |
      | Model     | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Summarize
      Model -[ai_languageModel]-> Summarize
      """
    And the trigger outputs the items:
      """
      [{"title": "Cats", "body": "Cats are independent and clean.", "views": 10}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Summarize" outputs:
      """
      [{"output": {"text": "Cats are good pets."}}]
      """
    And the mock OpenAI API received 1 chat request
    And chat request 1 has a "user" message with the text:
      """
      "Write a concise summary of the following:\n\n\n\"Cats\n\nCats are independent and clean.\"\n\n\nCONCISE SUMMARY:"
      """

  Scenario: Long text is summarized chunk by chunk, then combined
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "A summary."}]
      """
    And a workflow with nodes:
      | name      | type                  | parameters                                                                       |
      | Start     | manualTrigger         |                                                                                  |
      | Long      | set                   |                                                                                  |
      | Summarize | lc.chainSummarization | {"options": {}}                                                                  |
      | Model     | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Long" sets the fields:
      """
      {"text": "={{ 'word '.repeat(4000) }}"}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Long -> Summarize
      Model -[ai_languageModel]-> Summarize
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Summarize" outputs:
      """
      [{"output": {"text": "A summary."}}]
      """
    And the mock OpenAI API received 26 chat requests
    And chat request 26 contains a "user" message containing "A summary.\n\nA summary."

  Scenario: Refine improves the summary one chunk at a time
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": "First summary."},
        {"role": "assistant", "content": "Refined summary."}
      ]
      """
    And a workflow with nodes:
      | name      | type                  | parameters                                                                       |
      | Start     | manualTrigger         |                                                                                  |
      | Summarize | lc.chainSummarization |                                                                                  |
      | Model     | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Summarize" has parameters:
      """
      {"chunkSize": 30, "chunkOverlap": 0, "options": {"summarizationMethodAndPrompts": {"values": {"summarizationMethod": "refine"}}}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Summarize
      Model -[ai_languageModel]-> Summarize
      """
    And the trigger outputs the items:
      """
      [{"text": "The first part of the story.\n\nThe second part of it."}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Summarize" outputs:
      """
      [{"output": {"output_text": "Refined summary."}}]
      """
    And the mock OpenAI API received 2 chat requests
    And chat request 2 has a "user" message with the text:
      """
      "Your job is to produce a final summary\nWe have provided an existing summary up to a certain point: \"First summary.\"\nWe have the opportunity to refine the existing summary\n(only if needed) with some more context below.\n------------\n\"The second part of it.\"\n------------\n\nGiven the new context, refine the original summary\nIf the context isn't useful, return the original summary.\n\nREFINED SUMMARY:"
      """

  Scenario: Stuff puts every chunk into one custom prompt
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Done."}]
      """
    And a workflow with nodes:
      | name      | type                  | parameters                                                                       |
      | Start     | manualTrigger         |                                                                                  |
      | Summarize | lc.chainSummarization |                                                                                  |
      | Model     | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}} |
    And the node "Summarize" has parameters:
      """
      {"options": {"summarizationMethodAndPrompts": {"values": {"summarizationMethod": "stuff", "prompt": "Summarize in one line ({{braces}} stay): {text}"}}}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Summarize
      Model -[ai_languageModel]-> Summarize
      """
    And the trigger outputs the items:
      """
      [{"a": "Alpha", "b": "Beta"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 has a "user" message with the text:
      """
      "Summarize in one line ({braces} stay): Alpha\n\nBeta"
      """
