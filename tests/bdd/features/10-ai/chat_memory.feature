@spec-6.8 @phase-5
Feature: Postgres and Redis chat memory
  `memoryPostgresChat` and `memoryRedisChat` keep an agent's conversation
  per session outside the process, in the stores n8n's LangChain chat
  histories use, so a conversation survives restarts and can move between
  n8n and r8r: Postgres rows `(id, session_id, message JSONB)` in
  `n8n_chat_histories` (or the configured table), and a Redis list per
  session key, newest message first. The agent sends the last
  `contextWindowLength` exchanges before the new question.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """
    And a mock OpenAI API that replies in order:
      """
      [
        {"role": "assistant", "content": "Hi Ann"},
        {"role": "assistant", "content": "Your name is Ann"}
      ]
      """

  @requires-postgres
  Scenario: Postgres memory keeps the conversation between executions
    Given the Postgres table "bdd_chat_mem" does not exist
    And the credential "Local Postgres" of type "postgres" with the data:
      """
      {"host": "127.0.0.1", "port": 5432, "database": "postgres", "user": "postgres", "password": "postgres", "ssl": "disable"}
      """
    And a workflow with nodes:
      | name   | type                  | parameters                                                                                              |
      | Start  | manualTrigger         |                                                                                                         |
      | Agent  | lc.agent              |                                                                                                         |
      | Model  | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                        |
      | Memory | lc.memoryPostgresChat | {"sessionIdType": "customKey", "sessionKey": "session-1", "tableName": "bdd_chat_mem", "contextWindowLength": 5} |
    And the node "Agent" has parameters:
      """
      {"promptType": "define", "text": "={{ $json.question }}", "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Memory" uses the "postgres" credential "Local Postgres"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Memory -[ai_memory]-> Agent
      """
    And the trigger outputs the items:
      """
      [{"question": "My name is Ann"}]
      """
    When I execute the workflow
    And the trigger outputs the items:
      """
      [{"question": "What is my name?"}]
      """
    And I execute the workflow
    Then the execution succeeds
    And the node "Agent" outputs:
      """
      [{"output": "Your name is Ann"}]
      """
    And chat request 2 contains a "user" message containing "My name is Ann"
    And chat request 2 contains a "assistant" message containing "Hi Ann"
    And the Postgres table "bdd_chat_mem" holds for the session "session-1" the messages:
      """
      [
        {"type": "human", "content": "My name is Ann", "additional_kwargs": {}, "response_metadata": {}},
        {"type": "ai", "content": "Hi Ann", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}},
        {"type": "human", "content": "What is my name?", "additional_kwargs": {}, "response_metadata": {}},
        {"type": "ai", "content": "Your name is Ann", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}}
      ]
      """
    And the node "Memory" has run data on the "ai_memory" connection
    And the Postgres table "bdd_chat_mem" is dropped

  @requires-redis
  Scenario: Redis memory keeps the conversation in a list per session, with a TTL
    Given redis has no keys matching "bdd-redis-session"
    And the credential "Local Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379, "database": 0}
      """
    And a workflow with nodes:
      | name   | type               | parameters                                                                                                          |
      | Start  | manualTrigger      |                                                                                                                     |
      | Agent  | lc.agent           |                                                                                                                     |
      | Model  | lc.lmChatOpenAi    | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                                    |
      | Memory | lc.memoryRedisChat | {"sessionIdType": "customKey", "sessionKey": "bdd-redis-session", "sessionTTL": 3600, "contextWindowLength": 5}     |
    And the node "Agent" has parameters:
      """
      {"promptType": "define", "text": "={{ $json.question }}", "options": {}}
      """
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Memory" uses the "redis" credential "Local Redis"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Memory -[ai_memory]-> Agent
      """
    And the trigger outputs the items:
      """
      [{"question": "My name is Ann"}]
      """
    When I execute the workflow
    And the trigger outputs the items:
      """
      [{"question": "What is my name?"}]
      """
    And I execute the workflow
    Then the execution succeeds
    And chat request 2 contains a "user" message containing "My name is Ann"
    And chat request 2 contains a "assistant" message containing "Hi Ann"
    And the redis list "bdd-redis-session" holds the entries:
      """
      [
        {"type": "ai", "data": {"content": "Your name is Ann", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}}},
        {"type": "human", "data": {"content": "What is my name?", "additional_kwargs": {}, "response_metadata": {}}},
        {"type": "ai", "data": {"content": "Hi Ann", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}}},
        {"type": "human", "data": {"content": "My name is Ann", "additional_kwargs": {}, "response_metadata": {}}}
      ]
      """
    And the redis key "bdd-redis-session" has a ttl greater than 3000 seconds
    And redis has no keys matching "bdd-redis-session"

  @requires-postgres
  Scenario: A Postgres history written by n8n is replayed to the model
    Given the Postgres table "bdd_chat_mem_seed" does not exist
    And the Postgres table "bdd_chat_mem_seed" has for the session "seeded" the messages:
      """
      [
        {"type": "human", "content": "My name is Bob", "additional_kwargs": {}, "response_metadata": {}},
        {"type": "ai", "content": "Hello Bob", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}}
      ]
      """
    And the credential "Local Postgres" of type "postgres" with the data:
      """
      {"host": "127.0.0.1", "port": 5432, "database": "postgres", "user": "postgres", "password": "postgres", "ssl": "disable"}
      """
    And a workflow with nodes:
      | name   | type                  | parameters                                                                                                  |
      | Start  | manualTrigger         |                                                                                                             |
      | Agent  | lc.agent              | {"promptType": "define", "text": "What is my name?", "options": {}}                                         |
      | Model  | lc.lmChatOpenAi       | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}                            |
      | Memory | lc.memoryPostgresChat | {"sessionIdType": "customKey", "sessionKey": "seeded", "tableName": "bdd_chat_mem_seed", "contextWindowLength": 5} |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Memory" uses the "postgres" credential "Local Postgres"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Memory -[ai_memory]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 contains a "user" message containing "My name is Bob"
    And chat request 1 contains a "assistant" message containing "Hello Bob"
    And the Postgres table "bdd_chat_mem_seed" is dropped

  @requires-redis
  Scenario: A Redis history written by n8n is replayed to the model
    Given redis has no keys matching "bdd-redis-seeded"
    And the redis list "bdd-redis-seeded" has the entries pushed in order:
      """
      [
        {"type": "human", "data": {"content": "My name is Bob", "additional_kwargs": {}, "response_metadata": {}}},
        {"type": "ai", "data": {"content": "Hello Bob", "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}}}
      ]
      """
    And the credential "Local Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379, "database": 0}
      """
    And a workflow with nodes:
      | name   | type               | parameters                                                                                    |
      | Start  | manualTrigger      |                                                                                               |
      | Agent  | lc.agent           | {"promptType": "define", "text": "What is my name?", "options": {}}                           |
      | Model  | lc.lmChatOpenAi    | {"model": {"__rl": true, "mode": "list", "value": "gpt-4o-mini"}, "options": {}}              |
      | Memory | lc.memoryRedisChat | {"sessionIdType": "customKey", "sessionKey": "bdd-redis-seeded", "contextWindowLength": 5}    |
    And the node "Model" uses the "openAiApi" credential "Mock OpenAI"
    And the node "Memory" uses the "redis" credential "Local Redis"
    And the connections:
      """
      Start -> Agent
      Model -[ai_languageModel]-> Agent
      Memory -[ai_memory]-> Agent
      """
    When I execute the workflow
    Then the execution succeeds
    And chat request 1 contains a "user" message containing "My name is Bob"
    And chat request 1 contains a "assistant" message containing "Hello Bob"
    And redis has no keys matching "bdd-redis-seeded"
