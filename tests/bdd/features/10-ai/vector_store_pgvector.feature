@spec-6.8 @phase-5 @requires-postgres
Feature: Postgres PGVector Store with OpenAI embeddings
  `vectorStorePGVector` keeps documents in a PostgreSQL table with the
  pgvector extension, as LangChain's `PGVectorStore` does: the extension
  and table (`id uuid`, `text`, `metadata jsonb`, `embedding vector`) are
  created when missing, each document is a row, and a search orders the
  rows by the distance operator of the chosen strategy (`<=>` cosine,
  `<#>` inner product, `<->` euclidean). The `score` is that raw distance,
  so smaller is closer. Rows can be scoped to a named collection, and
  searches filtered by metadata. The embeddings API is a mock that returns
  fixed vectors per text.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And the credential "Local Postgres" of type "postgres" with the data:
      """
      {"host": "127.0.0.1", "port": 5432, "database": "postgres", "user": "postgres", "password": "postgres", "ssl": "disable"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "Fish swim": [0, 0, 1], "feline": [0.9, 0.1, 0]}
      """

  Scenario: Documents are inserted as rows, then the nearest ones are found
    Given the Postgres table "bdd_pgv_pets" does not exist
    And a workflow with nodes:
      | name       | type                         | parameters                                                                 |
      | Start      | manualTrigger                |                                                                            |
      | Insert     | lc.vectorStorePGVector       | {"mode": "insert", "tableName": "bdd_pgv_pets", "options": {}}             |
      | Once       | limit                        | {"maxItems": 1}                                                            |
      | Search     | lc.vectorStorePGVector       | {"mode": "load", "tableName": "bdd_pgv_pets", "prompt": "feline", "topK": 2, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                            |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                            |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Search" uses the "postgres" credential "Local Postgres"
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
    And the Postgres query "SELECT text, metadata, embedding::text AS embedding, length(id::text) AS id_length FROM bdd_pgv_pets ORDER BY text" returns:
      """
      [
        {"text": "Cats purr", "metadata": {"loc": {"lines": {"to": 1, "from": 1}}, "source": "blob", "blobType": "application/json"}, "embedding": "[1,0,0]", "id_length": 36},
        {"text": "Dogs bark", "metadata": {"loc": {"lines": {"to": 1, "from": 1}}, "source": "blob", "blobType": "application/json"}, "embedding": "[0,1,0]", "id_length": 36},
        {"text": "Fish swim", "metadata": {"loc": {"lines": {"to": 1, "from": 1}}, "source": "blob", "blobType": "application/json"}, "embedding": "[0,0,1]", "id_length": 36}
      ]
      """
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": {"loc": {"lines": {"to": 1, "from": 1}}, "source": "blob", "blobType": "application/json"}}, "score": 0.006116251198662548},
        {"document": {"pageContent": "Dogs bark", "metadata": {"loc": {"lines": {"to": 1, "from": 1}}, "source": "blob", "blobType": "application/json"}}, "score": 0.8895684677844125}
      ]
      """
    And the Postgres table "bdd_pgv_pets" is dropped

  Scenario: Metadata filters narrow the search, and the euclidean strategy scores by distance
    Given the Postgres table "bdd_pgv_filter" does not exist
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                                       |
      | Start      | manualTrigger                |                                                                                                                                                                  |
      | Insert     | lc.vectorStorePGVector       | {"mode": "insert", "tableName": "bdd_pgv_filter", "options": {}}                                                                                                 |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                                  |
      | Search     | lc.vectorStorePGVector       | {"mode": "load", "tableName": "bdd_pgv_filter", "prompt": "feline", "topK": 4, "includeDocumentMetadata": false, "options": {"distanceStrategy": "euclidean", "metadata": {"metadataValues": [{"name": "kind", "value": "pet"}]}}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                                  |
      | Loader     | lc.documentDefaultDataLoader | {"jsonMode": "expressionData", "jsonData": "={{ $json.text }}", "options": {"metadata": {"metadataValues": [{"name": "kind", "value": "={{ $json.kind }}"}]}}}     |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Search" uses the "postgres" credential "Local Postgres"
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
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr"}, "score": 0.14142137441313676},
        {"document": {"pageContent": "Dogs bark"}, "score": 1.2727921611791464}
      ]
      """
    And the Postgres table "bdd_pgv_filter" is dropped

  Scenario: Collections keep separate sets of documents in one table
    Given the Postgres table "bdd_pgv_coll" does not exist
    And the Postgres table "bdd_pgv_collections" does not exist
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                                                                         |
      | Start      | manualTrigger                |                                                                                                                                                                                                    |
      | Pets       | lc.vectorStorePGVector       | {"mode": "insert", "tableName": "bdd_pgv_coll", "options": {"collection": {"values": {"useCollection": true, "collectionName": "pets", "collectionTableName": "bdd_pgv_collections"}}}}             |
      | Other      | lc.vectorStorePGVector       | {"mode": "insert", "tableName": "bdd_pgv_coll", "options": {"collection": {"values": {"useCollection": true, "collectionName": "other", "collectionTableName": "bdd_pgv_collections"}}}}            |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                                                                    |
      | Search     | lc.vectorStorePGVector       | {"mode": "load", "tableName": "bdd_pgv_coll", "prompt": "feline", "topK": 4, "options": {"collection": {"values": {"useCollection": true, "collectionName": "other", "collectionTableName": "bdd_pgv_collections"}}}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                                                                    |
      | Loader     | lc.documentDefaultDataLoader | {"jsonMode": "expressionData", "jsonData": "={{ $json.text ?? $json.pageContent }}", "options": {}}                                                                                                |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Pets" uses the "postgres" credential "Local Postgres"
    And the node "Other" uses the "postgres" credential "Local Postgres"
    And the node "Search" uses the "postgres" credential "Local Postgres"
    And the connections:
      """
      Start -> Pets -> Other -> Once -> Search
      Embeddings -[ai_embedding]-> Pets
      Embeddings -[ai_embedding]-> Other
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Pets
      Loader -[ai_document]-> Other
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the Postgres query "SELECT c.name, count(*)::int AS n FROM bdd_pgv_coll v JOIN bdd_pgv_collections c ON c.uuid = v.collection_id GROUP BY c.name ORDER BY c.name" returns:
      """
      [{"name": "other", "n": 2}, {"name": "pets", "n": 2}]
      """
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": "$any"}, "score": 0.006116251198662548},
        {"document": {"pageContent": "Dogs bark", "metadata": "$any"}, "score": 0.8895684677844125}
      ]
      """
    And the Postgres table "bdd_pgv_coll" is dropped
    And the Postgres table "bdd_pgv_collections" is dropped

  Scenario: An agent searches the table as a tool and gets the documents with their row IDs
    Given the Postgres table "bdd_pgv_tool" does not exist
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"feline\"}"}}]},
        {"role": "assistant", "content": "Cats purr."}
      ]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                    |
      | Start      | manualTrigger                |                                                                                                               |
      | Insert     | lc.vectorStorePGVector       | {"mode": "insert", "tableName": "bdd_pgv_tool", "options": {}}                                                |
      | Once       | limit                        | {"maxItems": 1}                                                                                               |
      | Agent      | lc.agent                     | {"promptType": "define", "text": "What do cats do?", "options": {}}                                           |
      | Model      | lc.lmChatOpenAi              | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                              |
      | Pets       | lc.vectorStorePGVector       | {"mode": "retrieve-as-tool", "tableName": "bdd_pgv_tool", "toolDescription": "Facts about pets", "topK": 1, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                               |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                               |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Pets" uses the "postgres" credential "Local Postgres"
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
    And chat request 2 contains a "tool" message containing "{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"loc\\\":{\\\"lines\\\":{\\\"to\\\":1,\\\"from\\\":1}},\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\"},\\\"id\\\":\\\""
    And the node "Pets" has run data on the "ai_tool" connection
    And the Postgres table "bdd_pgv_tool" is dropped
