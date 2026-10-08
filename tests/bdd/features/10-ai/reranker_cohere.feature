@spec-6.8 @phase-5
Feature: Reranking vector store results with Cohere
  With "Rerank Results" on, a vector store sends the documents it found to
  its `ai_reranker` sub-node. Reranker Cohere (LangChain's `CohereRerank`)
  posts their text to Cohere's `/v1/rerank` and keeps the `topN` documents
  in Cohere's order. In load and tool modes the relevance score becomes the
  document's score; through the Vector Store Retriever the documents carry
  it as `metadata.relevanceScore`. Cohere is a mock here; `CO_API_URL`
  points the client at it.

  Background:
    Given a mock HTTP service
    And the environment variable "CO_API_URL" is "%{MOCK_URL}"
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And the credential "Mock Cohere" of type "cohereApi" with the data:
      """
      {"apiKey": "co-test-123"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "Fish swim": [0, 0, 1], "feline": [0.9, 0.1, 0]}
      """

  Scenario: Loaded documents come back in Cohere's order with its scores
    Given the mock service responds to POST "/v1/rerank" with status 200 and body:
      """
      {"id": "rr-1", "results": [{"index": 1, "relevance_score": 0.91}, {"index": 0, "relevance_score": 0.42}], "meta": {"api_version": {"version": "1"}, "billed_units": {"search_units": 1}}}
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                  |
      | Start      | manualTrigger                |                                                                                                                                             |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_load"}, "clearStore": true}                             |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                             |
      | Search     | lc.vectorStoreInMemory       | {"mode": "load", "prompt": "feline", "topK": 3, "useReranker": true, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_load"}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                             |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                             |
      | Reranker   | lc.rerankerCohere            | {"topN": 2}                                                                                                                                 |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Reranker" uses the "cohereApi" credential "Mock Cohere"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      Reranker -[ai_reranker]-> Search
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/rerank" had a JSON body matching:
      """
      {"model": "rerank-v3.5", "query": "feline", "documents": ["Cats purr", "Dogs bark", "Fish swim"], "top_n": 2}
      """
    And the last request to "/v1/rerank" had the header "Authorization" equal to "Bearer co-test-123"
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.91},
        {"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.42}
      ]
      """
    And the node "Reranker" has run data on the "ai_reranker" connection

  Scenario: The retriever hands the chain the reranked documents
    Given the mock service responds to POST "/v1/rerank" with status 200 and body:
      """
      {"id": "rr-1", "results": [{"index": 1, "relevance_score": 0.91}, {"index": 0, "relevance_score": 0.42}], "meta": {"api_version": {"version": "1"}, "billed_units": {"search_units": 1}}}
      """
    And a mock OpenAI API that replies in order:
      """
      [{"role": "assistant", "content": "Dogs bark."}]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                         |
      | Start      | manualTrigger                |                                                                                                                    |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_qa"}, "clearStore": true}      |
      | Once       | limit                        | {"maxItems": 1}                                                                                                    |
      | QA         | lc.chainRetrievalQa          | {"promptType": "define", "text": "feline", "options": {}}                                                          |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                   |
      | Retriever  | lc.retrieverVectorStore      | {"topK": 3}                                                                                                        |
      | Store      | lc.vectorStoreInMemory       | {"mode": "retrieve", "useReranker": true, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_qa"}}   |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                    |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                    |
      | Reranker   | lc.rerankerCohere            | {"topN": 2}                                                                                                        |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Reranker" uses the "cohereApi" credential "Mock Cohere"
    And the connections:
      """
      Start -> Insert -> Once -> QA
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      Model -[ai_languageModel]-> QA
      Retriever -[ai_retriever]-> QA
      Store -[ai_vectorStore]-> Retriever
      Embeddings -[ai_embedding]-> Store
      Reranker -[ai_reranker]-> Store
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 contains a "system" message containing "Context: Dogs bark\n\nCats purr"

  Scenario: An agent's vector store tool returns the reranked documents
    Given the mock service responds to POST "/v1/rerank" with status 200 and body:
      """
      {"id": "rr-1", "results": [{"index": 1, "relevance_score": 0.91}, {"index": 0, "relevance_score": 0.42}], "meta": {"api_version": {"version": "1"}, "billed_units": {"search_units": 1}}}
      """
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"feline\"}"}}]},
        {"role": "assistant", "content": "Dogs bark."}
      ]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                                     |
      | Start      | manualTrigger                |                                                                                                                                                                |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_tool"}, "clearStore": true}                                                |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                                |
      | Agent      | lc.agent                     | {"promptType": "define", "text": "What do cats do?", "options": {}}                                                                                            |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                               |
      | Pets       | lc.vectorStoreInMemory       | {"mode": "retrieve-as-tool", "toolDescription": "Facts about pets", "topK": 3, "useReranker": true, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_tool"}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                                |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                                                |
      | Reranker   | lc.rerankerCohere            | {"topN": 2}                                                                                                                                                    |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Reranker" uses the "cohereApi" credential "Mock Cohere"
    And the connections:
      """
      Start -> Insert -> Once -> Agent
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      Model -[ai_languageModel]-> Agent
      Pets -[ai_tool]-> Agent
      Embeddings -[ai_embedding]-> Pets
      Reranker -[ai_reranker]-> Pets
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 2 has a "tool" message with the text:
      """
      "[{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Dogs bark\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc\\\":{\\\"lines\\\":{\\\"from\\\":1,\\\"to\\\":1}}}}\"},{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc\\\":{\\\"lines\\\":{\\\"from\\\":1,\\\"to\\\":1}}}}\"}]"
      """

  Scenario: Cohere's error reaches the node
    Given the mock service responds to POST "/v1/rerank" with status 401 and body:
      """
      {"message": "invalid api token"}
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                   |
      | Start      | manualTrigger                |                                                                                                                                              |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_err"}, "clearStore": true}                               |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                              |
      | Search     | lc.vectorStoreInMemory       | {"mode": "load", "prompt": "feline", "topK": 3, "useReranker": true, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_rerank_err"}}  |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                              |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                              |
      | Reranker   | lc.rerankerCohere            | {"topN": 2}                                                                                                                                  |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Reranker" uses the "cohereApi" credential "Mock Cohere"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      Reranker -[ai_reranker]-> Search
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Search" failed with an error containing "Status code: 401"
    And the node "Search" failed with an error containing "invalid api token"
