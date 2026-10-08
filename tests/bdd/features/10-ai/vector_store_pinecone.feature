@spec-6.8 @phase-5
Feature: Pinecone Vector Store with OpenAI embeddings
  `vectorStorePinecone` keeps documents in a Pinecone index, as LangChain's
  `PineconeStore` does through the Pinecone client: inserting checks that
  the index exists (`GET /indexes`), finds its host (`GET /indexes/{name}`),
  optionally clears the namespace, and upserts vectors with random UUIDs
  whose metadata is the document's, flattened (`loc.lines.from`), plus the
  text under `text`. A search queries the index host and turns matches back
  into documents (dropping matches scored 0). Pinecone is a mock here;
  `PINECONE_CONTROLLER_HOST` points the client at it.

  Background:
    Given a mock HTTP service
    And the environment variable "PINECONE_CONTROLLER_HOST" is "%{MOCK_URL}"
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And the credential "Mock Pinecone" of type "pineconeApi" with the data:
      """
      {"apiKey": "pc-test-123"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "feline": [0.5, 0.25, 0]}
      """
    And the mock service responds to GET "/indexes" with status 200 and body:
      """
      {"indexes": [{"name": "bdd-index", "dimension": 3, "metric": "cosine", "host": "%{MOCK_URL}", "spec": {"serverless": {"cloud": "aws", "region": "us-east-1"}}, "status": {"ready": true, "state": "Ready"}, "deletion_protection": "disabled"}]}
      """
    And the mock service responds to GET "/indexes/bdd-index" with status 200 and body:
      """
      {"name": "bdd-index", "dimension": 3, "metric": "cosine", "host": "%{MOCK_URL}", "spec": {"serverless": {"cloud": "aws", "region": "us-east-1"}}, "status": {"ready": true, "state": "Ready"}, "deletion_protection": "disabled"}
      """
    And the mock service responds to POST "/vectors/upsert" with status 200 and body:
      """
      {"upsertedCount": 2}
      """
    And the mock service responds to POST "/vectors/delete" with status 200 and body:
      """
      {}
      """
    And the mock service responds to POST "/query" with status 200 and body:
      """
      {"matches": [
        {"id": "id-cats", "score": 0.89, "values": [], "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1, "text": "Cats purr"}},
        {"id": "id-dogs", "score": 0.44, "values": [], "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1, "text": "Dogs bark"}},
        {"id": "id-zero", "score": 0, "values": [], "metadata": {"text": "Never shown"}}
      ], "namespace": "", "usage": {"readUnits": 6}}
      """

  Scenario: Documents are upserted with flattened metadata, then found
    Given a workflow with nodes:
      | name       | type                         | parameters                                                                                                                     |
      | Start      | manualTrigger                |                                                                                                                                |
      | Insert     | lc.vectorStorePinecone       | {"mode": "insert", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "options": {}}                        |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                |
      | Search     | lc.vectorStorePinecone       | {"mode": "load", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "prompt": "feline", "topK": 3, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "pineconeApi" credential "Mock Pinecone"
    And the node "Search" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/vectors/upsert" had a JSON body matching:
      """
      {"vectors": [
        {"id": "$regex:^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$", "values": [1, 0, 0], "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1, "text": "Cats purr"}},
        {"id": "$string", "values": [0, 1, 0], "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1, "text": "Dogs bark"}}
      ], "namespace": ""}
      """
    And the last request to "/vectors/upsert" had the header "Api-Key" equal to "pc-test-123"
    And the last request to "/vectors/upsert" had the header "X-Pinecone-Api-Version" equal to "2025-01"
    And the last request to "/indexes" had the header "Api-Key" equal to "pc-test-123"
    And the last request to "/query" had a JSON body matching:
      """
      {"namespace": "", "topK": 3, "includeMetadata": true, "vector": [0.5, 0.25, 0]}
      """
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1}}, "score": 0.89},
        {"document": {"pageContent": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc.lines.from": 1, "loc.lines.to": 1}}, "score": 0.44}
      ]
      """

  Scenario: A namespace is cleared before inserting, and searches are filtered by metadata
    Given a workflow with nodes:
      | name       | type                         | parameters                                                                                                                                                                                                       |
      | Start      | manualTrigger                |                                                                                                                                                                                                                  |
      | Insert     | lc.vectorStorePinecone       | {"mode": "insert", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "options": {"pineconeNamespace": "pets", "clearNamespace": true}}                                                       |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                                                                                                  |
      | Search     | lc.vectorStorePinecone       | {"mode": "load", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "prompt": "feline", "topK": 1, "includeDocumentMetadata": false, "options": {"pineconeNamespace": "pets", "metadata": {"metadataValues": [{"name": "source", "value": "blob"}]}}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                                                                                                  |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                                                                                                  |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "pineconeApi" credential "Mock Pinecone"
    And the node "Search" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Insert -> Once -> Search
      Embeddings -[ai_embedding]-> Insert
      Embeddings -[ai_embedding]-> Search
      Loader -[ai_document]-> Insert
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}, {"text": "Dogs bark"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/vectors/delete" had a JSON body matching:
      """
      {"deleteAll": true, "namespace": "pets"}
      """
    And the last request to "/vectors/upsert" had a JSON body matching:
      """
      {"namespace": "pets"}
      """
    And the last request to "/query" had a JSON body matching:
      """
      {"namespace": "pets", "topK": 1, "filter": {"source": "blob"}, "includeMetadata": true}
      """
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr"}, "score": 0.89},
        {"document": {"pageContent": "Dogs bark"}, "score": 0.44}
      ]
      """

  Scenario: Inserting into an index that doesn't exist fails before embedding
    Given a workflow with nodes:
      | name       | type                         | parameters                                                                                                   |
      | Start      | manualTrigger                |                                                                                                              |
      | Insert     | lc.vectorStorePinecone       | {"mode": "insert", "pineconeIndex": {"__rl": true, "mode": "id", "value": "missing-index"}, "options": {}}    |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                              |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                              |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "pineconeApi" credential "Mock Pinecone"
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
    And the node "Insert" failed with an error containing "Index missing-index not found"
    And the mock embeddings API received 0 requests

  Scenario: An agent searches the index as a tool and gets the documents with their IDs
    Given a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"feline\"}"}}]},
        {"role": "assistant", "content": "Cats purr."}
      ]
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                                                                         |
      | Start      | manualTrigger          |                                                                                                                                                    |
      | Agent      | lc.agent               | {"promptType": "define", "text": "What do cats do?", "options": {}}                                                                                |
      | Model      | lc.lmChatOpenAi        | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                   |
      | Pets       | lc.vectorStorePinecone | {"mode": "retrieve-as-tool", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "toolDescription": "Facts about pets", "topK": 1, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                                    |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Pets" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Pets -[ai_tool]-> Agent
      Embeddings -[ai_embedding]-> Pets
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "Cats purr."}]
      """
    And chat request 2 has a "tool" message with the text:
      """
      "[{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc.lines.from\\\":1,\\\"loc.lines.to\\\":1},\\\"id\\\":\\\"id-cats\\\"}\"},{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Dogs bark\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\",\\\"blobType\\\":\\\"application/json\\\",\\\"loc.lines.from\\\":1,\\\"loc.lines.to\\\":1},\\\"id\\\":\\\"id-dogs\\\"}\"}]"
      """
    And the last request to "/query" had a JSON body matching:
      """
      {"namespace": "", "topK": 1, "includeMetadata": true}
      """

  Scenario: A rejected API key surfaces Pinecone's error without leaking the key
    Given the mock service responds to GET "/indexes/locked-index" with status 401 and body:
      """
      {"error": {"code": "UNAUTHENTICATED", "message": "Invalid API Key"}, "status": 401}
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                                                      |
      | Start      | manualTrigger          |                                                                                                                                 |
      | Search     | lc.vectorStorePinecone | {"mode": "load", "pineconeIndex": {"__rl": true, "mode": "list", "value": "locked-index"}, "prompt": "feline", "topK": 1, "options": {}}    |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                 |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Search" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Search
      Embeddings -[ai_embedding]-> Search
      """
    When I execute the workflow
    Then the execution fails
    And the node "Search" failed with an error containing "The API key you provided was rejected while calling http://127.0.0.1"
    And the node "Search" failed with an error containing "/indexes/locked-index. Please check your configuration values and try again."
    And the execution data does not contain "pc-test-123"

  Scenario: Update mode replaces a vector under its ID
    Each input item is read whole as JSON and must give one document,
    which is embedded and upserted under the item's ID.
    Given a workflow with nodes:
      | name       | type                   | parameters                                                                                                    |
      | Start      | manualTrigger          |                                                                                                               |
      | Update     | lc.vectorStorePinecone | {"mode": "update", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "id": "={{ $json.id }}"} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                               |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Update" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Update
      Embeddings -[ai_embedding]-> Update
      """
    And the trigger outputs the items:
      """
      [{"id": 7, "text": "Cats purr"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"metadata": {"source": "blob", "blobType": "application/json"}, "pageContent": "Cats purr"}]
      """
    And the last request to "/vectors/upsert" had a JSON body matching:
      """
      {"vectors": [{"id": 7, "values": [1, 0, 0], "metadata": {"source": "blob", "blobType": "application/json", "text": "Cats purr"}}], "namespace": ""}
      """

  Scenario: Update mode needs exactly one document per item
    Given a workflow with nodes:
      | name       | type                   | parameters                                                                                              |
      | Start      | manualTrigger          |                                                                                                         |
      | Update     | lc.vectorStorePinecone | {"mode": "update", "pineconeIndex": {"__rl": true, "mode": "list", "value": "bdd-index"}, "id": "id-cats"} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                         |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Update" uses the "pineconeApi" credential "Mock Pinecone"
    And the connections:
      """
      Start -> Update
      Embeddings -[ai_embedding]-> Update
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr", "more": "Dogs bark"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Update" failed with an error containing "Single document per item expected"
    And the mock embeddings API received 0 requests
