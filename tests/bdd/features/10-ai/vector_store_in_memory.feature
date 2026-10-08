@spec-6.8 @phase-5
Feature: Simple Vector Store with OpenAI embeddings
  `vectorStoreInMemory` keeps documents and their embeddings in memory
  under a key. In insert mode each item goes through the connected
  document loader (the Default Data Loader: the item's strings as
  documents, split into chunks) and is embedded with the connected
  embeddings sub-node; the node outputs the stored documents. In load mode
  ("Get Many") the prompt is embedded and the `topK` most similar
  documents come out with their cosine similarity as `score`. The
  embeddings API here is a mock that returns fixed vectors per text.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "Fish swim": [0, 0, 1], "feline": [0.9, 0.1, 0]}
      """

  Scenario: Documents are inserted, then the closest ones are found
    Given a workflow with nodes:
      | name       | type                         | parameters                                                                                                          |
      | Start      | manualTrigger                |                                                                                                                     |
      | Insert     | lc.vectorStoreInMemory       | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_pets"}, "clearStore": true}            |
      | Once       | limit                        | {"maxItems": 1}                                                                                                     |
      | Search     | lc.vectorStoreInMemory       | {"mode": "load", "prompt": "feline", "topK": 2, "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_pets"}}   |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                     |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                     |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
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
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.9938837341719244},
        {"document": {"pageContent": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.1104315305900972}
      ]
      """
    And the last request to "/v1/embeddings" had a JSON body matching:
      """
      {"model": "text-embedding-3-small", "input": "feline", "encoding_format": "base64"}
      """
    And the node "Embeddings" has run data on the "ai_embedding" connection
    And the node "Loader" has run data on the "ai_document" connection

  Scenario: A text splitter sub-node splits documents into chunks before embedding
    Given a workflow with nodes:
      | name       | type                                           | parameters                                                                                                |
      | Start      | manualTrigger                                  |                                                                                                           |
      | Insert     | lc.vectorStoreInMemory                         | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_split"}, "clearStore": true} |
      | Embeddings | lc.embeddingsOpenAi                            | {"options": {}}                                                                                           |
      | Loader     | lc.documentDefaultDataLoader                   | {"textSplittingMode": "custom", "options": {}}                                                            |
      | Splitter   | lc.textSplitterRecursiveCharacterTextSplitter  | {"chunkSize": 12, "chunkOverlap": 0, "options": {}}                                                       |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Insert
      Embeddings -[ai_embedding]-> Insert
      Loader -[ai_document]-> Insert
      Splitter -[ai_textSplitter]-> Loader
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr\nDogs bark"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [
        {"metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "pageContent": "Cats purr"},
        {"metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 2, "to": 2}}}, "pageContent": "Dogs bark"}
      ]
      """
    And the mock embeddings API received 1 request
    And the last request to "/v1/embeddings" had a JSON body matching:
      """
      {"input": ["Cats purr", "Dogs bark"]}
      """
    And the node "Splitter" has run data on the "ai_textSplitter" connection

  Scenario: A metadata filter in the workflow JSON is ignored, as the node has no such option
    n8n only reads parameters the node describes, and the Simple Vector
    Store's load mode has no options.
    Given a workflow with nodes:
      | name       | type                   | parameters                                                                                                                                                                 |
      | Start      | manualTrigger          |                                                                                                                                                                            |
      | Insert     | lc.vectorStoreInMemory | {"mode": "insert", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_filter"}, "clearStore": true}                                                                  |
      | Once       | limit                  | {"maxItems": 1}                                                                                                                                                            |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                                                      |
      | Search     | lc.vectorStoreInMemory | {"mode": "load", "prompt": "feline", "memoryKey": {"__rl": true, "mode": "list", "value": "bdd_filter"}, "options": {"metadata": {"metadataValues": [{"name": "kind", "value": "pet"}]}}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                                                            |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Search" outputs:
      """
      [{"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.9938837341719244}]
      """
