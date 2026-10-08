@spec-6.8 @phase-5 @requires-qdrant
Feature: Qdrant Vector Store with OpenAI embeddings
  `vectorStoreQdrant` keeps documents as points in a Qdrant collection, as
  LangChain's `QdrantVectorStore` does over Qdrant's REST API: a missing
  collection is created (from the "Collection Config" option, or with the
  embedding size, found by embedding "test", and cosine distance), each
  document is a point with a random UUID and a `{content, metadata}`
  payload, and a search runs Qdrant's query API, scored by the
  collection's metric. Searches take Qdrant's own filter syntax. The
  embeddings API is a mock that returns fixed vectors per text.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And the credential "Local Qdrant" of type "qdrantApi" with the data:
      """
      {"qdrantUrl": "http://127.0.0.1:6333", "apiKey": ""}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "Fish swim": [0, 0, 1], "feline": [0.9, 0.1, 0], "test": [1, 1, 1]}
      """

  Scenario: A new collection is created, filled, and searched
    Given the Qdrant collection "bdd_qd_pets" does not exist
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                      |
      | Start      | manualTrigger                |                                                                                                                                 |
      | Insert     | lc.vectorStoreQdrant         | {"mode": "insert", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_pets"}, "options": {}}                      |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                 |
      | Search     | lc.vectorStoreQdrant         | {"mode": "load", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_pets"}, "prompt": "feline", "topK": 2, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                 |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                 |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "qdrantApi" credential "Local Qdrant"
    And the node "Search" uses the "qdrantApi" credential "Local Qdrant"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}, {"text": "Fish swim"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [
        {"metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "pageContent": "Cats purr"},
        {"metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "pageContent": "Dogs bark"},
        {"metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "pageContent": "Fish swim"}
      ]
      """
    And the Qdrant collection "bdd_qd_pets" has 3 points of 3 dimensions compared by "Cosine"
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.9938837},
        {"document": {"pageContent": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.11043153}
      ]
      """
    And the mock embeddings API received 3 requests

  Scenario: A collection config, custom payload keys and a Qdrant filter
    Given the Qdrant collection "bdd_qd_filter" does not exist
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                                                                                  |
      | Start      | manualTrigger                |                                                                                                                                                                                                             |
      | Insert     | lc.vectorStoreQdrant         | {"mode": "insert", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_filter"}, "options": {"collectionConfig": {"vectors": {"size": 3, "distance": "Euclid"}}, "contentPayloadKey": "body", "metadataPayloadKey": "meta"}} |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                                                                             |
      | Search     | lc.vectorStoreQdrant         | {"mode": "load", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_filter"}, "prompt": "feline", "topK": 4, "includeDocumentMetadata": false, "options": {"searchFilterJson": {"must": [{"key": "meta.kind", "match": {"value": "pet"}}]}, "contentPayloadKey": "body", "metadataPayloadKey": "meta"}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                                                                             |
      | Loader     | lc.documentDefaultDataLoader | {"jsonMode": "expressionData", "jsonData": "={{ $json.text }}", "options": {"metadata": {"metadataValues": [{"name": "kind", "value": "={{ $json.kind }}"}]}}}                                                 |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "qdrantApi" credential "Local Qdrant"
    And the node "Search" uses the "qdrantApi" credential "Local Qdrant"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr", "kind": "pet"}, {"text": "Dogs bark", "kind": "pet"}, {"text": "Fish swim", "kind": "food"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the Qdrant collection "bdd_qd_filter" has 3 points of 3 dimensions compared by "Euclid"
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr"}, "score": 0.14142138},
        {"document": {"pageContent": "Dogs bark"}, "score": 1.2727922}
      ]
      """
    And the mock embeddings API received 2 requests

  Scenario: Qdrant's error reaches the node when the vectors don't fit the collection
    Given the Qdrant collection "bdd_qd_dims" exists with 2-dimensional cosine vectors
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                   |
      | Start      | manualTrigger                |                                                                                                              |
      | Insert     | lc.vectorStoreQdrant         | {"mode": "insert", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_dims"}, "options": {}}   |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                              |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                              |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "qdrantApi" credential "Local Qdrant"
    And the connections:
      """
      Start -> Insert
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Insert" failed with an error containing "400 Bad Request: Wrong input: Vector dimension error: expected dim: 2, got 3"

  Scenario: An agent searches the collection as a tool and gets the documents with their point IDs
    Given the Qdrant collection "bdd_qd_tool" does not exist
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"feline\"}"}}]},
        {"role": "assistant", "content": "Cats purr."}
      ]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                       |
      | Start      | manualTrigger                |                                                                                                                                                  |
      | Insert     | lc.vectorStoreQdrant         | {"mode": "insert", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_tool"}, "options": {}}                                       |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                  |
      | Agent      | lc.agent                     | {"promptType": "define", "text": "What do cats do?", "options": {}}                                                                              |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                 |
      | Pets       | lc.vectorStoreQdrant         | {"mode": "retrieve-as-tool", "qdrantCollection": {"__rl": true, "mode": "id", "value": "bdd_qd_tool"}, "toolDescription": "Facts about pets", "topK": 1, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                  |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                                  |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "qdrantApi" credential "Local Qdrant"
    And the node "Pets" uses the "qdrantApi" credential "Local Qdrant"
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
    And chat request 2 contains a "tool" message containing "{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc\\\":{\\\"lines\\\":{\\\"from\\\":1,\\\"to\\\":1}}},\\\"id\\\":\\\""
    And the node "Pets" has run data on the "ai_tool" connection
