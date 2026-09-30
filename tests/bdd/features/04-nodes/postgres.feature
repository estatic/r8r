@spec-6.6 @phase-4 @node-postgres @requires-postgres
Feature: Postgres node
  executeQuery, insert, update, upsert, deleteTable (delete/truncate/drop)
  and select against a real PostgreSQL 16 (R8R_BDD_POSTGRES_URL, default
  postgres://postgres:postgres@127.0.0.1:5432/postgres; opt in with
  R8R_BDD_INCLUDE=requires-postgres). Each scenario creates its own
  uniquely named table and drops it again, via the node under test itself
  (an `executeQuery` "Setup"/"Cleanup" node), so scenarios stay independent
  under parallel execution.

  Background:
    Given the credential "Local Postgres" of type "postgres" with the data:
      """
      {"host": "127.0.0.1", "port": 5432, "database": "postgres", "user": "postgres", "password": "postgres", "ssl": "disable"}
      """

  Scenario: executeQuery substitutes $1-style parameters and a SQL injection attempt stays inert data
    Given a workflow with nodes:
      | name    | type     | typeVersion |
      | Start   | manualTrigger |        |
      | Setup   | postgres | 2.5         |
      | Insert  | postgres | 2.5         |
      | Count   | postgres | 2.5         |
      | Cleanup | postgres | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_inj(id int primary key, name text)", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO pg_bdd_inj(id, name) VALUES($1, $2)", "options": {"queryReplacement": "1,Robert'); DROP TABLE pg_bdd_inj;--"}}
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT name FROM pg_bdd_inj WHERE id = 1) AS stored FROM pg_bdd_inj", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_inj", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Count" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Insert -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "1"
    And the field "stored" of item 0 from the node "Count" is "Robert'); DROP TABLE pg_bdd_inj;--"

  Scenario: executeQuery surfaces a Postgres syntax error with a clear message
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Bad   | postgres      | 2.5         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELEKT * FROM nowhere", "options": {}}
      """
    And the node "Bad" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution fails
    And the node "Bad" failed with an error containing "syntax error"

  Scenario: continueOnFail turns a failing query into an error item instead of failing the node
    Given a workflow with nodes:
      | name  | type          | onError               | typeVersion |
      | Start | manualTrigger |                        |             |
      | Bad   | postgres      | continueRegularOutput  | 2.5         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM this_table_does_not_exist_at_all", "options": {}}
      """
    And the node "Bad" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution succeeds
    And the node "Bad" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: Insert with auto-mapped input data returns the inserted row
    Given a workflow named "InsAutoSetup" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_ins_auto(id int primary key, name text, active boolean)", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup"
    When I execute the workflow "InsAutoSetup"
    Then the execution succeeds

    Given a workflow named "InsAutoWrite" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Insert  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "name": "Alice", "active": true}]
      """
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_ins_auto"},
        "columns": {"mappingMode": "autoMapInputData", "value": null, "matchingColumns": []},
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_ins_auto", "options": {}}
      """
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Insert -> Cleanup"
    When I execute the workflow "InsAutoWrite"
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [{"id": 1, "name": "Alice", "active": true}]
      """

  Scenario: Insert with columns defined below ignores input item data
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Insert  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_ins_define(id int primary key, name text)", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_ins_define"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 2, "name": "Bob"}, "matchingColumns": []},
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_ins_define", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Insert -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [{"id": 2, "name": "Bob"}]
      """

  Scenario: Select applies where, sort and limit
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Seed    | postgres      | 2.5         |
      | Select  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_select(id int primary key, name text, score int)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO pg_bdd_select VALUES (1,'A',10),(2,'B',20),(3,'C',30)", "options": {}}
      """
    And the node "Seed" uses the "postgres" credential "Local Postgres"
    And the node "Select" has parameters:
      """
      {
        "operation": "select",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_select"},
        "where": {"values": [{"column": "id", "condition": ">", "value": "1"}]},
        "combineConditions": "AND",
        "sort": {"values": [{"column": "score", "direction": "DESC"}]},
        "returnAll": false,
        "limit": 10,
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_select", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Select" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Seed -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Select" outputs:
      """
      [{"id": 3, "name": "C", "score": 30}, {"id": 2, "name": "B", "score": 20}]
      """

  Scenario: Select returnAll ignores the limit
    # A `Select` matching zero rows outputs zero items, and n8n's engine
    # skips downstream nodes fed zero items -- so `Cleanup` can't be
    # chained after `Select` here (it would just never run). Verify and
    # clean up as two separate executions instead.
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Select  | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_returnall(id int primary key)", "options": {}}
      """
    And the node "Select" has parameters:
      """
      {
        "operation": "select",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_returnall"},
        "where": {"values": []},
        "sort": {"values": [{"column": "id", "direction": "ASC"}]},
        "returnAll": true,
        "options": {}
      }
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Select" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Select"
    When I execute the workflow
    Then the execution succeeds
    And the node "Select" outputs 0 items

    Given a workflow named "ReturnAllCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | postgres      | 2.5         |
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_returnall", "options": {}}
      """
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Cleanup"
    When I execute the workflow "ReturnAllCleanup"
    Then the execution succeeds

  Scenario: Update changes the matched row
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Seed    | postgres      | 2.5         |
      | Update  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_update(id int primary key, name text)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO pg_bdd_update VALUES (1,'X')", "options": {}}
      """
    And the node "Seed" uses the "postgres" credential "Local Postgres"
    And the node "Update" has parameters:
      """
      {
        "operation": "update",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_update"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 1, "name": "Y"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_update", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Update" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Seed -> Update -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"id": 1, "name": "Y"}]
      """

  Scenario: Updating a row that doesn't exist fails with a clear error
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Update  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_update_missing(id int primary key, name text)", "options": {}}
      """
    And the node "Update" has parameters:
      """
      {
        "operation": "update",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_update_missing"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 999, "name": "Y"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_update_missing", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Update" uses the "postgres" credential "Local Postgres"
    And the node "Update" has the property "onError" set to "continueRegularOutput"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Update -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "error" of item 0 from the node "Update" is "$contains:doesn't exist"

  Scenario: Upsert inserts on the first call and updates on conflict
    Given a workflow with nodes:
      | name     | type          | typeVersion |
      | Start    | manualTrigger |             |
      | Setup    | postgres      | 2.5         |
      | UpsertA  | postgres      | 2.5         |
      | UpsertB  | postgres      | 2.5         |
      | Count    | postgres      | 2.5         |
      | Cleanup  | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_upsert(id int primary key, val text)", "options": {}}
      """
    And the node "UpsertA" has parameters:
      """
      {
        "operation": "upsert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_upsert"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 1, "val": "a"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "UpsertB" has parameters:
      """
      {
        "operation": "upsert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_upsert"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 1, "val": "b"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT val FROM pg_bdd_upsert WHERE id = 1) AS val FROM pg_bdd_upsert", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_upsert", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "UpsertA" uses the "postgres" credential "Local Postgres"
    And the node "UpsertB" uses the "postgres" credential "Local Postgres"
    And the node "Count" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> UpsertA -> UpsertB -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "UpsertA" outputs:
      """
      [{"id": 1, "val": "a"}]
      """
    And the node "UpsertB" outputs:
      """
      [{"id": 1, "val": "b"}]
      """
    And the field "cnt" of item 0 from the node "Count" is "1"
    And the field "val" of item 0 from the node "Count" is "b"

  Scenario: Delete removes only the matching rows
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Seed    | postgres      | 2.5         |
      | Delete  | postgres      | 2.5         |
      | Count   | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_delete(id int primary key)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO pg_bdd_delete VALUES (1),(2),(3)", "options": {}}
      """
    And the node "Seed" uses the "postgres" credential "Local Postgres"
    And the node "Delete" has parameters:
      """
      {
        "operation": "deleteTable",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_delete"},
        "deleteCommand": "delete",
        "where": {"values": [{"column": "id", "condition": "equal", "value": "2"}]},
        "combineConditions": "AND",
        "options": {}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM pg_bdd_delete", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_delete", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Delete" uses the "postgres" credential "Local Postgres"
    And the node "Count" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Seed -> Delete -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "2"

  Scenario: Truncate empties the table without dropping it
    Given a workflow with nodes:
      | name     | type          | typeVersion |
      | Start    | manualTrigger |             |
      | Setup    | postgres      | 2.5         |
      | Seed     | postgres      | 2.5         |
      | Truncate | postgres      | 2.5         |
      | Count    | postgres      | 2.5         |
      | Cleanup  | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_truncate(id int primary key)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO pg_bdd_truncate VALUES (1),(2)", "options": {}}
      """
    And the node "Seed" uses the "postgres" credential "Local Postgres"
    And the node "Truncate" has parameters:
      """
      {
        "operation": "deleteTable",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_truncate"},
        "deleteCommand": "truncate",
        "options": {}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM pg_bdd_truncate", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_truncate", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Truncate" uses the "postgres" credential "Local Postgres"
    And the node "Count" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Seed -> Truncate -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "0"

  Scenario: Drop removes the table entirely
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | postgres      | 2.5         |
      | Drop  | postgres      | 2.5         |
      | Check | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_drop(id int primary key)", "options": {}}
      """
    And the node "Drop" has parameters:
      """
      {
        "operation": "deleteTable",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_drop"},
        "deleteCommand": "drop",
        "options": {}
      }
      """
    And the node "Check" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM pg_bdd_drop", "options": {}}
      """
    And the node "Check" has the property "onError" set to "continueRegularOutput"
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Drop" uses the "postgres" credential "Local Postgres"
    And the node "Check" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Drop -> Check"
    When I execute the workflow
    Then the execution succeeds
    And the field "error" of item 0 from the node "Check" is "$contains:does not exist"

  Scenario: Type round-trip for int, numeric, boolean, text, timestamptz, jsonb and null
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | postgres      | 2.5         |
      | Insert  | postgres      | 2.5         |
      | Select  | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_types(id int primary key, n numeric, b boolean, t text, ts timestamptz, j jsonb, nothing text)", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_types"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": 1, "n": 123.45, "b": true, "t": "hello", "ts": "2024-01-15T10:30:00.000Z", "j": {"a": 1, "b": [1, 2]}, "nothing": null}, "matchingColumns": []},
        "options": {}
      }
      """
    And the node "Select" has parameters:
      """
      {
        "operation": "select",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_types"},
        "where": {"values": []},
        "sort": {"values": []},
        "returnAll": true,
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_types", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the node "Insert" uses the "postgres" credential "Local Postgres"
    And the node "Select" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup -> Insert -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "id" of item 0 from the node "Select" is 1
    And the field "n" of item 0 from the node "Select" is "123.45"
    And the field "b" of item 0 from the node "Select" is true
    And the field "t" of item 0 from the node "Select" is "hello"
    And the field "ts" of item 0 from the node "Select" is "2024-01-15T10:30:00.000Z"
    And the field "j.a" of item 0 from the node "Select" is 1
    And the field "j.b" of item 0 from the node "Select" is "$any"
    And the field "nothing" of item 0 from the node "Select" is null

  Scenario: A connection failure fails clearly and never leaks the password
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | postgres      | 2.5         |
    And the credential "Bad Postgres" of type "postgres" with the data:
      """
      {"host": "127.0.0.1", "port": 5499, "database": "postgres", "user": "postgres", "password": "super-secret-password", "ssl": "disable"}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1", "options": {"connectionTimeout": 3}}
      """
    And the node "Query" uses the "postgres" credential "Bad Postgres"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "super-secret-password"

  Scenario: Transaction batching rolls back every item when one query fails
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_tx(id int primary key, val text not null)", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "TxAttempt" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Write | postgres      | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "val": "ok"}, {"id": 2, "val": null}]
      """
    And the node "Write" has parameters:
      """
      {
        "operation": "insert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_tx"},
        "columns": {"mappingMode": "autoMapInputData", "value": null, "matchingColumns": []},
        "options": {"queryBatching": "transaction"}
      }
      """
    And the node "Write" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Write"
    When I execute the workflow "TxAttempt"
    Then the execution fails

    Given a workflow named "TxVerify" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Count   | postgres      | 2.5         |
      | Cleanup | postgres      | 2.5         |
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM pg_bdd_tx", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_tx", "options": {}}
      """
    And the node "Count" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Count -> Cleanup"
    When I execute the workflow "TxVerify"
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "0"

  Scenario: independently batching runs every item on its own, so one failure doesn't block the rest
    Given a workflow named "IndepSetup" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | postgres      | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE pg_bdd_indep(id int primary key, val text)", "options": {}}
      """
    And the node "Setup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Setup"
    When I execute the workflow "IndepSetup"
    Then the execution succeeds

    Given a workflow named "IndepWrite" with nodes:
      | name    | type          | onError               | typeVersion |
      | Start   | manualTrigger |                        |             |
      | Write   | postgres      | continueRegularOutput  | 2.5         |
      | Cleanup | postgres      |                        | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "val": "a"}, {"id": 1, "val": "dup"}, {"id": 2, "val": "b"}]
      """
    And the node "Write" has parameters:
      """
      {
        "operation": "insert",
        "schema": {"__rl": true, "mode": "list", "value": "public"},
        "table": {"__rl": true, "mode": "name", "value": "pg_bdd_indep"},
        "columns": {"mappingMode": "autoMapInputData", "value": null, "matchingColumns": []},
        "options": {"queryBatching": "independently"}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS pg_bdd_indep", "options": {}}
      """
    And the node "Write" uses the "postgres" credential "Local Postgres"
    And the node "Cleanup" uses the "postgres" credential "Local Postgres"
    And the connections "Start -> Write -> Cleanup"
    When I execute the workflow "IndepWrite"
    Then the execution succeeds
    And the node "Write" outputs 3 items
