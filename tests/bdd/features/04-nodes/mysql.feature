@spec-6.6 @phase-4 @node-mysql @requires-mysql
Feature: MySQL node
  executeQuery, insert, update, upsert, deleteTable (delete/truncate/drop)
  and select against a real MySQL 8 (Docker `r8r-bdd-mysql`, 127.0.0.1:3306,
  user root / password mysql, database r8r; opt in with
  R8R_BDD_INCLUDE=requires-mysql). Each scenario creates its own uniquely
  named table and drops it again, via the node under test itself (an
  `executeQuery` "Setup"/"Cleanup" node), so scenarios stay independent
  under parallel execution.

  Unlike the Postgres node, MySQL's `insert`/`update`/`upsert` never use
  `RETURNING` (MySQL doesn't support it here), so a successful write
  outputs `{"success": true}`, not the affected row -- scenarios verify
  writes with a follow-up `select`.

  Background:
    Given the credential "Local MySQL" of type "mySql" with the data:
      """
      {"host": "127.0.0.1", "port": 3306, "database": "r8r", "user": "root", "password": "mysql", "ssl": false}
      """

  Scenario: executeQuery substitutes $1-style parameters and a SQL injection attempt stays inert data
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Insert  | mySql         | 2.5         |
      | Count   | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_inj(id INT PRIMARY KEY, name VARCHAR(200))", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_inj(id, name) VALUES($1, $2)", "options": {"queryReplacement": "1,Robert'); DROP TABLE my_bdd_inj;--"}}
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT name FROM my_bdd_inj WHERE id = 1) AS stored FROM my_bdd_inj", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_inj", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Insert" uses the "mySql" credential "Local MySQL"
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Insert -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "1"
    And the field "stored" of item 0 from the node "Count" is "Robert'); DROP TABLE my_bdd_inj;--"

  Scenario: executeQuery surfaces a MySQL syntax error with a clear message
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Bad   | mySql         | 2.5         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELEKT * FROM nowhere", "options": {}}
      """
    And the node "Bad" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution fails
    And the node "Bad" failed with an error containing "SQL syntax"

  Scenario: continueOnFail turns a failing query into an error item instead of failing the node
    Given a workflow with nodes:
      | name  | type          | onError               | typeVersion |
      | Start | manualTrigger |                        |             |
      | Bad   | mySql         | continueRegularOutput  | 2.5         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM this_table_does_not_exist_at_all", "options": {}}
      """
    And the node "Bad" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution succeeds
    And the node "Bad" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: Insert with auto-mapped input data (bulk single-batch insert)
    Given a workflow named "MyInsAutoSetup" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_ins_auto(id INT PRIMARY KEY, name VARCHAR(50), active TINYINT(1))", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup"
    When I execute the workflow "MyInsAutoSetup"
    Then the execution succeeds

    Given a workflow named "MyInsAutoWrite" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Insert  | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "name": "Alice", "active": true}]
      """
    And the node "Insert" has parameters:
      """
      {"operation": "insert", "table": {"__rl": true, "mode": "name", "value": "my_bdd_ins_auto"}, "dataMode": "autoMapInputData", "options": {}}
      """
    And the node "Select" has parameters:
      """
      {"operation": "select", "table": {"__rl": true, "mode": "name", "value": "my_bdd_ins_auto"}, "where": {"values": []}, "sort": {"values": []}, "returnAll": true, "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_ins_auto", "options": {}}
      """
    And the node "Insert" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Insert -> Select -> Cleanup"
    When I execute the workflow "MyInsAutoWrite"
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [{"success": true}]
      """
    And the node "Select" outputs:
      """
      [{"id": 1, "name": "Alice", "active": true}]
      """

  Scenario: Insert with values defined below ignores input item data
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Insert  | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_ins_define(id INT PRIMARY KEY, name VARCHAR(50))", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_ins_define"},
        "dataMode": "defineBelow",
        "valuesToSend": {"values": [{"column": "id", "value": 2}, {"column": "name", "value": "Bob"}]},
        "options": {}
      }
      """
    And the node "Select" has parameters:
      """
      {"operation": "select", "table": {"__rl": true, "mode": "name", "value": "my_bdd_ins_define"}, "where": {"values": []}, "sort": {"values": []}, "returnAll": true, "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_ins_define", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Insert" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Insert -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Select" outputs:
      """
      [{"id": 2, "name": "Bob"}]
      """

  Scenario: Select applies where, sort and limit
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Seed    | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_select(id INT PRIMARY KEY, name VARCHAR(10), score INT)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_select VALUES (1,'A',10),(2,'B',20),(3,'C',30)", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Select" has parameters:
      """
      {
        "operation": "select",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_select"},
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
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_select", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
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
    # chained after `Select` here. Verify and clean up as two separate
    # executions instead.
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_returnall(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Select" has parameters:
      """
      {"operation": "select", "table": {"__rl": true, "mode": "name", "value": "my_bdd_returnall"}, "where": {"values": []}, "sort": {"values": [{"column": "id", "direction": "ASC"}]}, "returnAll": true, "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Select"
    When I execute the workflow
    Then the execution succeeds
    And the node "Select" outputs 0 items

    Given a workflow named "MyReturnAllCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mySql         | 2.5         |
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_returnall", "options": {}}
      """
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MyReturnAllCleanup"
    Then the execution succeeds

  Scenario: Update changes the matched row
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Seed    | mySql         | 2.5         |
      | Update  | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_update(id INT PRIMARY KEY, name VARCHAR(10))", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_update VALUES (1,'X')", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Update" has parameters:
      """
      {
        "operation": "update",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_update"},
        "dataMode": "defineBelow",
        "columnToMatchOn": "id",
        "valueToMatchOn": "1",
        "valuesToSend": {"values": [{"column": "name", "value": "Y"}]},
        "options": {}
      }
      """
    And the node "Select" has parameters:
      """
      {"operation": "select", "table": {"__rl": true, "mode": "name", "value": "my_bdd_update"}, "where": {"values": []}, "sort": {"values": []}, "returnAll": true, "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_update", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Update" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Seed -> Update -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"success": true}]
      """
    And the node "Select" outputs:
      """
      [{"id": 1, "name": "Y"}]
      """

  Scenario: Updating a row that doesn't exist succeeds without error (MySQL doesn't pre-check existence)
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Update  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_update_missing(id INT PRIMARY KEY, name VARCHAR(10))", "options": {}}
      """
    And the node "Update" has parameters:
      """
      {
        "operation": "update",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_update_missing"},
        "dataMode": "defineBelow",
        "columnToMatchOn": "id",
        "valueToMatchOn": "999",
        "valuesToSend": {"values": [{"column": "name", "value": "Y"}]},
        "options": {}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_update_missing", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Update" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Update -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"success": true}]
      """

  Scenario: Upsert inserts on the first call and updates on conflict
    Given a workflow with nodes:
      | name     | type          | typeVersion |
      | Start    | manualTrigger |             |
      | Setup    | mySql         | 2.5         |
      | UpsertA  | mySql         | 2.5         |
      | UpsertB  | mySql         | 2.5         |
      | Count    | mySql         | 2.5         |
      | Cleanup  | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_upsert(id INT PRIMARY KEY, val VARCHAR(10))", "options": {}}
      """
    And the node "UpsertA" has parameters:
      """
      {
        "operation": "upsert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_upsert"},
        "dataMode": "defineBelow",
        "columnToMatchOn": "id",
        "valueToMatchOn": "1",
        "valuesToSend": {"values": [{"column": "val", "value": "a"}]},
        "options": {}
      }
      """
    And the node "UpsertB" has parameters:
      """
      {
        "operation": "upsert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_upsert"},
        "dataMode": "defineBelow",
        "columnToMatchOn": "id",
        "valueToMatchOn": "1",
        "valuesToSend": {"values": [{"column": "val", "value": "b"}]},
        "options": {}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT val FROM my_bdd_upsert WHERE id = 1) AS val FROM my_bdd_upsert", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_upsert", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "UpsertA" uses the "mySql" credential "Local MySQL"
    And the node "UpsertB" uses the "mySql" credential "Local MySQL"
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> UpsertA -> UpsertB -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "1"
    And the field "val" of item 0 from the node "Count" is "b"

  Scenario: Delete removes only the matching rows
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Seed    | mySql         | 2.5         |
      | Delete  | mySql         | 2.5         |
      | Count   | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_delete(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_delete VALUES (1),(2),(3)", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Delete" has parameters:
      """
      {
        "operation": "deleteTable",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_delete"},
        "deleteCommand": "delete",
        "where": {"values": [{"column": "id", "condition": "equal", "value": "2"}]},
        "combineConditions": "AND",
        "options": {}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM my_bdd_delete", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_delete", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Delete" uses the "mySql" credential "Local MySQL"
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Seed -> Delete -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "2"

  Scenario: Truncate empties the table without dropping it
    Given a workflow with nodes:
      | name     | type          | typeVersion |
      | Start    | manualTrigger |             |
      | Setup    | mySql         | 2.5         |
      | Seed     | mySql         | 2.5         |
      | Truncate | mySql         | 2.5         |
      | Count    | mySql         | 2.5         |
      | Cleanup  | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_truncate(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_truncate VALUES (1),(2)", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Truncate" has parameters:
      """
      {"operation": "deleteTable", "table": {"__rl": true, "mode": "name", "value": "my_bdd_truncate"}, "deleteCommand": "truncate", "options": {}}
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM my_bdd_truncate", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_truncate", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Truncate" uses the "mySql" credential "Local MySQL"
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Seed -> Truncate -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "0"

  Scenario: Drop removes the table entirely
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | mySql         | 2.5         |
      | Drop  | mySql         | 2.5         |
      | Check | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_drop(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Drop" has parameters:
      """
      {"operation": "deleteTable", "table": {"__rl": true, "mode": "name", "value": "my_bdd_drop"}, "deleteCommand": "drop", "options": {}}
      """
    And the node "Check" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM my_bdd_drop", "options": {}}
      """
    And the node "Check" has the property "onError" set to "continueRegularOutput"
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Drop" uses the "mySql" credential "Local MySQL"
    And the node "Check" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Drop -> Check"
    When I execute the workflow
    Then the execution succeeds
    And the field "error" of item 0 from the node "Check" is "$contains:doesn't exist"

  Scenario: Type round-trip for int, bigint, decimal, datetime, json, boolean (tinyint) and null
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Insert  | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_types(id INT PRIMARY KEY, big BIGINT, amt DECIMAL(12,2), dt DATETIME, j JSON, flag TINYINT(1), nothing VARCHAR(10))", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_types"},
        "dataMode": "defineBelow",
        "valuesToSend": {"values": [
          {"column": "id", "value": 1},
          {"column": "big", "value": 9223372036854775807},
          {"column": "amt", "value": 123.45},
          {"column": "dt", "value": "2024-01-15 10:30:00"},
          {"column": "j", "value": "{\"a\": 1, \"b\": [1, 2]}"},
          {"column": "flag", "value": true},
          {"column": "nothing", "value": null}
        ]},
        "options": {}
      }
      """
    And the node "Select" has parameters:
      """
      {"operation": "select", "table": {"__rl": true, "mode": "name", "value": "my_bdd_types"}, "where": {"values": []}, "sort": {"values": []}, "returnAll": true, "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_types", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Insert" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Insert -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "id" of item 0 from the node "Select" is 1
    And the field "big" of item 0 from the node "Select" is "9223372036854775807"
    And the field "amt" of item 0 from the node "Select" is "123.45"
    And the field "dt" of item 0 from the node "Select" is "2024-01-15 10:30:00"
    And the field "j.a" of item 0 from the node "Select" is 1
    And the field "j.b" of item 0 from the node "Select" is "$any"
    And the field "flag" of item 0 from the node "Select" is true
    And the field "nothing" of item 0 from the node "Select" is null

  Scenario: A connection failure fails clearly and never leaks the password
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | mySql         | 2.5         |
    And the credential "Bad MySQL Host" of type "mySql" with the data:
      """
      {"host": "127.0.0.1", "port": 3399, "database": "r8r", "user": "root", "password": "super-secret-password", "ssl": false}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1", "options": {}}
      """
    And the node "Query" uses the "mySql" credential "Bad MySQL Host"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "super-secret-password"

  Scenario: A wrong password fails clearly
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | mySql         | 2.5         |
    And the credential "Bad MySQL Password" of type "mySql" with the data:
      """
      {"host": "127.0.0.1", "port": 3306, "database": "r8r", "user": "root", "password": "definitely-wrong", "ssl": false}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1", "options": {}}
      """
    And the node "Query" uses the "mySql" credential "Bad MySQL Password"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails

  Scenario: Transaction batching rolls back every item when one query fails
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_tx(id INT PRIMARY KEY, val VARCHAR(10) NOT NULL)", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MyTxAttempt" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Write | mySql         | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "val": "ok"}, {"id": 2, "val": null}]
      """
    And the node "Write" has parameters:
      """
      {
        "operation": "insert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_tx"},
        "dataMode": "autoMapInputData",
        "options": {"queryBatching": "transaction"}
      }
      """
    And the node "Write" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Write"
    When I execute the workflow "MyTxAttempt"
    Then the execution fails

    Given a workflow named "MyTxVerify" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Count   | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM my_bdd_tx", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_tx", "options": {}}
      """
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Count -> Cleanup"
    When I execute the workflow "MyTxVerify"
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "0"

  Scenario: independently batching runs every item on its own, so one failure doesn't block the rest
    Given a workflow named "MyIndepSetup" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_indep(id INT PRIMARY KEY, val VARCHAR(10))", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup"
    When I execute the workflow "MyIndepSetup"
    Then the execution succeeds

    Given a workflow named "MyIndepWrite" with nodes:
      | name    | type          | onError               | typeVersion |
      | Start   | manualTrigger |                        |             |
      | Write   | mySql         | continueRegularOutput  | 2.5         |
      | Cleanup | mySql         |                        | 2.5         |
    And the trigger outputs the items:
      """
      [{"id": 1, "val": "a"}, {"id": 1, "val": "dup"}, {"id": 2, "val": "b"}]
      """
    And the node "Write" has parameters:
      """
      {
        "operation": "insert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_indep"},
        "dataMode": "autoMapInputData",
        "options": {"queryBatching": "independently"}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_indep", "options": {}}
      """
    And the node "Write" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Write -> Cleanup"
    When I execute the workflow "MyIndepWrite"
    Then the execution succeeds
    And the node "Write" outputs 3 items

  Scenario: skipOnConflict (INSERT IGNORE) avoids a duplicate-key error
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Seed    | mySql         | 2.5         |
      | Insert  | mySql         | 2.5         |
      | Count   | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_ignore(id INT PRIMARY KEY, val VARCHAR(10))", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_ignore VALUES (1,'first')", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Insert" has parameters:
      """
      {
        "operation": "insert",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_ignore"},
        "dataMode": "defineBelow",
        "valuesToSend": {"values": [{"column": "id", "value": 1}, {"column": "val", "value": "second"}]},
        "options": {"skipOnConflict": true, "queryBatching": "independently"}
      }
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT val FROM my_bdd_ignore WHERE id = 1) AS val FROM my_bdd_ignore", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_ignore", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Insert" uses the "mySql" credential "Local MySQL"
    And the node "Count" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Seed -> Insert -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is "1"
    And the field "val" of item 0 from the node "Count" is "first"

  Scenario: selectDistinct removes duplicate rows and outputColumns limits the projection
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Seed    | mySql         | 2.5         |
      | Select  | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_distinct(id INT PRIMARY KEY, val VARCHAR(10))", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO my_bdd_distinct VALUES (1,'x'),(2,'x'),(3,'y')", "options": {}}
      """
    And the node "Seed" uses the "mySql" credential "Local MySQL"
    And the node "Select" has parameters:
      """
      {
        "operation": "select",
        "table": {"__rl": true, "mode": "name", "value": "my_bdd_distinct"},
        "where": {"values": []},
        "sort": {"values": [{"column": "val", "direction": "ASC"}]},
        "returnAll": true,
        "options": {"outputColumns": ["val"], "selectDistinct": true}
      }
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_distinct", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Select" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Seed -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Select" outputs:
      """
      [{"val": "x"}, {"val": "y"}]
      """

  Scenario: detailedOutput reports the executed SQL and result data instead of a plain confirmation
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | mySql         | 2.5         |
      | Query   | mySql         | 2.5         |
      | Cleanup | mySql         | 2.5         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE my_bdd_detailed(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1 AS one", "options": {"detailedOutput": true}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS my_bdd_detailed", "options": {}}
      """
    And the node "Setup" uses the "mySql" credential "Local MySQL"
    And the node "Query" uses the "mySql" credential "Local MySQL"
    And the node "Cleanup" uses the "mySql" credential "Local MySQL"
    And the connections "Start -> Setup -> Query -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "sql" of item 0 from the node "Query" is "$contains:SELECT 1"
    And the field "data[0].one" of item 0 from the node "Query" is "1"
