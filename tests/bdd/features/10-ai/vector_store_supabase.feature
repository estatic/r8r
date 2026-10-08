@spec-6.8 @phase-5
Feature: Supabase Vector Store with OpenAI embeddings
  `vectorStoreSupabase` keeps documents in a Supabase table through
  PostgREST (`{host}/rest/v1`), as LangChain's `SupabaseVectorStore` does
  with supabase-js: rows of `content`, `embedding` and `metadata` are
  upserted 500 at a time, and a search calls the table's match function
  over RPC (`match_documents` unless "Query Name" says otherwise) with the
  query embedding, the metadata filter (`{}` when none) and `match_count`,
  scoring each row by the `similarity` it returns. The service role key is
  sent as `apikey` and as the bearer token. Supabase is a mock here.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And the credential "Mock Supabase" of type "supabaseApi" with the data:
      """
      {"host": "%{MOCK_URL}", "serviceRole": "sb-secret-123"}
      """
    And a mock OpenAI embeddings API with the vectors:
      """
      {"Cats purr": [1, 0, 0], "Dogs bark": [0, 1, 0], "feline": [0.5, 0.25, 0]}
      """

  Scenario: Rows are upserted through PostgREST, then matched over RPC
    Given the mock service responds to POST "/rest/v1/bdd_docs" with status 201 and body:
      """
      [{"id": 1, "content": "Cats purr"}, {"id": 2, "content": "Dogs bark"}]
      """
    And the mock service responds to POST "/rest/v1/rpc/match_documents" with status 200 and body:
      """
      [
        {"id": 1, "content": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "similarity": 0.894427190999916},
        {"id": 2, "content": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}, "similarity": 0.447213595499958}
      ]
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                                                     |
      | Start      | manualTrigger                |                                                                                                                                |
      | Insert     | lc.vectorStoreSupabase       | {"mode": "insert", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "options": {}}                             |
      | Once       | limit                        | {"maxItems": 1}                                                                                                                |
      | Search     | lc.vectorStoreSupabase       | {"mode": "load", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "prompt": "feline", "topK": 2, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                                                |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                                                |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "supabaseApi" credential "Mock Supabase"
    And the node "Search" uses the "supabaseApi" credential "Mock Supabase"
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
    And the last request to "/rest/v1/bdd_docs" had a JSON body matching:
      """
      [
        {"content": "Cats purr", "embedding": [1, 0, 0], "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}},
        {"content": "Dogs bark", "embedding": [0, 1, 0], "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}
      ]
      """
    And the last request to "/rest/v1/bdd_docs" had the query parameter "columns" equal to '"content","embedding","metadata"'
    And the last request to "/rest/v1/bdd_docs" had the query parameter "select" equal to "*"
    And the last request to "/rest/v1/bdd_docs" had the header "Prefer" equal to "resolution=merge-duplicates,return=representation"
    And the last request to "/rest/v1/bdd_docs" had the header "apikey" equal to "sb-secret-123"
    And the last request to "/rest/v1/bdd_docs" had the header "Authorization" equal to "Bearer sb-secret-123"
    And the last request to "/rest/v1/bdd_docs" had the header "Content-Profile" equal to "public"
    And the last request to "/rest/v1/rpc/match_documents" had a JSON body matching:
      """
      {"query_embedding": [0.5, 0.25, 0], "filter": {}, "match_count": 2}
      """
    And the node "Search" outputs:
      """
      [
        {"document": {"pageContent": "Cats purr", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.894427190999916},
        {"document": {"pageContent": "Dogs bark", "metadata": {"source": "blob", "blobType": "application/json", "loc": {"lines": {"from": 1, "to": 1}}}}, "score": 0.447213595499958}
      ]
      """

  Scenario: A custom match function gets the metadata filter
    Given the mock service responds to POST "/rest/v1/rpc/match_pets" with status 200 and body:
      """
      [{"id": 1, "content": "Cats purr", "metadata": {"kind": "pet"}, "similarity": 0.9}]
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                                                                                                                                              |
      | Start      | manualTrigger          |                                                                                                                                                                                                                         |
      | Search     | lc.vectorStoreSupabase | {"mode": "load", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "prompt": "feline", "topK": 5, "options": {"queryName": "match_pets", "metadata": {"metadataValues": [{"name": "kind", "value": "pet"}]}}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                                                                                                         |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Search" uses the "supabaseApi" credential "Mock Supabase"
    And the connections:
      """
      Start -> Search
      Embeddings -[ai_embedding]-> Search
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/rest/v1/rpc/match_pets" had a JSON body matching:
      """
      {"query_embedding": [0.5, 0.25, 0], "filter": {"kind": "pet"}, "match_count": 5}
      """
    And the node "Search" outputs:
      """
      [{"document": {"pageContent": "Cats purr", "metadata": {"kind": "pet"}}, "score": 0.9}]
      """

  Scenario: A missing table is reported with PostgREST's message
    Given the mock service responds to POST "/rest/v1/missing" with status 404 and body:
      """
      {"code": "PGRST205", "details": null, "hint": "Perhaps you meant the table 'public.documents'", "message": "Could not find the table 'public.missing' in the schema cache"}
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                           |
      | Start      | manualTrigger                |                                                                                                      |
      | Insert     | lc.vectorStoreSupabase       | {"mode": "insert", "tableName": {"__rl": true, "mode": "id", "value": "missing"}, "options": {}}      |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                      |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                      |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "supabaseApi" credential "Mock Supabase"
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
    And the node "Insert" failed with an error containing "Error inserting: Could not find the table 'public.missing' in the schema cache 404 Not Found"

  Scenario: A 404 without a message is n8n's "Table not found"
    Given the mock service responds to POST "/rest/v1/missing" with status 404 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name       | type                         | parameters                                                                                           |
      | Start      | manualTrigger                |                                                                                                      |
      | Insert     | lc.vectorStoreSupabase       | {"mode": "insert", "tableName": {"__rl": true, "mode": "id", "value": "missing"}, "options": {}}      |
      | Embeddings | lc.embeddingsOpenAi          | {"options": {}}                                                                                      |
      | Loader     | lc.documentDefaultDataLoader | {"options": {}}                                                                                      |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Insert" uses the "supabaseApi" credential "Mock Supabase"
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
    And the node "Insert" failed with an error containing "Table missing not found"

  Scenario: A missing match function is reported with PostgREST's code and details
    Given the mock service responds to POST "/rest/v1/rpc/match_documents" with status 404 and body:
      """
      {"code": "PGRST202", "details": "Searched for the function public.match_documents with parameters filter, match_count, query_embedding", "hint": null, "message": "Could not find the function public.match_documents(filter, match_count, query_embedding) in the schema cache"}
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                                                     |
      | Start      | manualTrigger          |                                                                                                                                |
      | Search     | lc.vectorStoreSupabase | {"mode": "load", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "prompt": "feline", "topK": 2, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Search" uses the "supabaseApi" credential "Mock Supabase"
    And the connections:
      """
      Start -> Search
      Embeddings -[ai_embedding]-> Search
      """
    When I execute the workflow
    Then the execution fails
    And the node "Search" failed with an error containing "Error searching for documents: PGRST202 Could not find the function public.match_documents(filter, match_count, query_embedding) in the schema cache Searched for the function public.match_documents with parameters filter, match_count, query_embedding"
    And the execution data does not contain "sb-secret-123"

  Scenario: An agent searches the table as a tool
    Given the mock service responds to POST "/rest/v1/rpc/match_documents" with status 200 and body:
      """
      [{"id": 1, "content": "Cats purr", "metadata": {"source": "blob"}, "similarity": 0.89}]
      """
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "Pets", "arguments": "{\"input\": \"feline\"}"}}]},
        {"role": "assistant", "content": "Cats purr."}
      ]
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                                                                        |
      | Start      | manualTrigger          |                                                                                                                                                   |
      | Agent      | lc.agent               | {"promptType": "define", "text": "What do cats do?", "options": {}}                                                                               |
      | Model      | lc.lmChatOpenAi        | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                                                  |
      | Pets       | lc.vectorStoreSupabase | {"mode": "retrieve-as-tool", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "toolDescription": "Facts about pets", "topK": 1, "options": {}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                                                                   |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Pets" uses the "supabaseApi" credential "Mock Supabase"
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
      "[{\"type\":\"text\",\"text\":\"{\\\"pageContent\\\":\\\"Cats purr\\\",\\\"metadata\\\":{\\\"source\\\":\\\"blob\\\"}}\"}]"
      """
    And the last request to "/rest/v1/rpc/match_documents" had a JSON body matching:
      """
      {"filter": {}, "match_count": 1}
      """

  Scenario: Update mode upserts a row under its ID
    Given the mock service responds to POST "/rest/v1/bdd_docs" with status 201 and body:
      """
      [{"id": 7, "content": "Cats purr"}]
      """
    And a workflow with nodes:
      | name       | type                   | parameters                                                                                              |
      | Start      | manualTrigger          |                                                                                                         |
      | Update     | lc.vectorStoreSupabase | {"mode": "update", "tableName": {"__rl": true, "mode": "list", "value": "bdd_docs"}, "id": "7", "options": {}} |
      | Embeddings | lc.embeddingsOpenAi    | {"options": {}}                                                                                         |
    And the node "Embeddings" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Update" uses the "supabaseApi" credential "Mock Supabase"
    And the connections:
      """
      Start -> Update
      Embeddings -[ai_embedding]-> Update
      """
    And the trigger outputs the items:
      """
      [{"text": "Cats purr"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"metadata": {"source": "blob", "blobType": "application/json"}, "pageContent": "Cats purr"}]
      """
    And the last request to "/rest/v1/bdd_docs" had a JSON body matching:
      """
      [{"id": "7", "content": "Cats purr", "embedding": [1, 0, 0], "metadata": {"source": "blob", "blobType": "application/json"}}]
      """
    And the last request to "/rest/v1/bdd_docs" had the query parameter "columns" equal to '"id","content","embedding","metadata"'
