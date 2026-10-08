@spec-6.8 @phase-5
Feature: Retrieval: Q&A Chain, Vector Store Retriever and vector store tools
  A vector store in "retrieve" mode feeds a Vector Store Retriever, which
  feeds the Question and Answer Chain: the question is embedded, the
  `topK` closest documents are joined into the system prompt's
  `{context}`, and the model answers. In "retrieve-as-tool" mode the store
  is an agent tool taking an `input` query and returning the documents
  found. Documents are inserted first, in the same execution (the Simple
  Vector Store lives in memory).

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "Fish swim": [0, 0, 1], "What do cats do?": [0.9, 0.1, 0], "cats": [1, 0.1, 0]}
      """

  Scenario: The Q&A Chain answers from the retrieved documents
    Given a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "They purr."}]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                 |
      | Start      | manualTrigger                |                                                                                                            |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_qa"}, "clearStore": true}     |
      | Once       | limit                        | {"maxItems": 1}                                                                                            |
      | QA         | lc.chainRetrievalQa             | {"promptType": "define", "text": "What do cats do?", "options": {}}                                        |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                           |
      | Retriever  | lc.retrieverVectorStore      | {"topK": 2}                                                                                                |
      | Store      | lc.vectorStoreInMemory       | {"mode": "retrieve", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_qa"}}                      |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                            |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                            |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Insert -> Once -> QA
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      Model -[ai_languageModel]-> QA
      Retriever -[ai_retriever]-> QA
      Store -[ai_vectorStore]-> Retriever
      Embeddings -[ai_embedding]-> Store
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "QA" outputs:
      """
      [{"response": "They purr."}]
      """
    And chat request 1 has a "system" message with the text:
      """
      "You are an assistant for question-answering tasks. Use the following pieces of retrieved context to answer the question.\nIf you don't know the answer, just say that you don't know, don't try to make up an answer.\n----------------\nContext: Cats purr\n\nDogs bark"
      """
    And chat request 1 has a "user" message with the content "What do cats do?"
    And the node "Store" has run data on the "ai_vectorStore" connection

  Scenario: An agent searches the store as a tool
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"cats\"}"}}]},
        {"role": "assistant", "content": "Cats purr."}
      ]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                         |
      | Start      | manualTrigger                |                                                                                                                                                    |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_tool"}, "clearStore": true}                                           |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                    |
      | Agent      | lc.agent                     | {"promptType": "define", "text": "What do cats do?", "options": {}}                                                                                |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                   |
      | Pets       | lc.vectorStoreInMemory       | {"mode": "retrieve-as-tool", "toolDescription": "Facts about pets", "topK": 1, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_tool"}}    |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                    |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                                    |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Insert -> Once -> Agent
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      Model -[ai_languageModel]-> Agent
      Pets -[ai_tool]-> Agent
      Embeddings -[ai_embedding]-> Pets
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "Cats purr."}]
      """
    And chat request 1 offers exactly the tool "Pets":
      """
      {"type":"function","function":{"name":"Pets","description":"Facts about pets","parameters":{"type":"object","properties":{"input":{"type":"string","description":"Query to search for. Required"}},"required":["input"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"},"strict":false}}
      """
    And chat request 2 has a "tool" message with the text:
      """
      "[{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc\\\":{\\\"lines\\\":{\\\"from\\\":1,\\\"to\\\":1}}}}\"}]"
      """
    And the node "Pets" has run data on the "ai_tool" connection
