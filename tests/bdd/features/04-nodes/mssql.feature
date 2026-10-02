@spec-6.6 @phase-4 @node-mssql @requires-mssql
Feature: Microsoft SQL node
  executeQuery, insert, update and delete against a real SQL Server
  (Docker `r8r-bdd-mssql`, 127.0.0.1:1433, user sa / password
  R8r_Passw0rd!, database r8r; opt in with R8R_BDD_INCLUDE=requires-mssql).
  Each scenario creates its own uniquely named table and drops it again,
  via the node under test itself (an `executeQuery` "Setup"/"Cleanup"
  node), so scenarios stay independent under parallel execution.

  Unlike the Postgres/MySQL nodes, this node has no `select` operation --
  reads go through `executeQuery` -- and `table`/`columns`/`updateKey`/
  `deleteKey` are plain strings, not resource locators. `insert`/`update`
  echo back the input items unchanged on success (no `RETURNING`-style
  readback); scenarios verify writes with a follow-up `executeQuery`.

  Background:
    Given the credential "Local MSSQL" of type "microsoftSql" with the data:
      """
      {"server": "127.0.0.1", "port": 1433, "database": "r8r", "user": "sa", "password": "R8r_Passw0rd!", "tls": false}
      """

  Scenario: executeQuery substitutes $1-style parameters and a SQL injection attempt stays inert data
    Given a workflow with nodes:
      | name    | type         | typeVersion |
      | Start   | manualTrigger |            |
      | Setup   | microsoftSql | 1.2         |
      | Insert  | microsoftSql | 1.2         |
      | Count   | microsoftSql | 1.2         |
      | Cleanup | microsoftSql | 1.2         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE ms_bdd_inj(id INT PRIMARY KEY, name NVARCHAR(200))", "options": {}}
      """
    And the node "Insert" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO ms_bdd_inj(id, name) VALUES($1, $2)", "options": {"queryReplacement": "1,Robert'); DROP TABLE ms_bdd_inj;--"}}
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt, (SELECT name FROM ms_bdd_inj WHERE id = 1) AS stored_val FROM ms_bdd_inj", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_inj", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Insert" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Count" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup -> Insert -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is 1
    And the field "stored_val" of item 0 from the node "Count" is "Robert'); DROP TABLE ms_bdd_inj;--"

  Scenario: executeQuery flattens multiple result sets into one output array
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Multi | microsoftSql  | 1.2         |
    And the node "Multi" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1 AS a; SELECT 2 AS b", "options": {}}
      """
    And the node "Multi" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Multi"
    When I execute the workflow
    Then the execution succeeds
    And the node "Multi" outputs:
      """
      [{"a": 1}, {"b": 2}]
      """

  Scenario: executeQuery surfaces a SQL Server error with a clear message
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Bad   | microsoftSql  | 1.2         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM ms_bdd_does_not_exist_at_all", "options": {}}
      """
    And the node "Bad" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution fails
    And the node "Bad" failed with an error containing "Invalid object name"

  Scenario: continueOnFail turns a failing executeQuery item into an error item instead of failing the node
    Given a workflow with nodes:
      | name  | type          | onError               | typeVersion |
      | Start | manualTrigger |                        |             |
      | Bad   | microsoftSql  | continueRegularOutput  | 1.2         |
    And the node "Bad" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM ms_bdd_does_not_exist_at_all", "options": {}}
      """
    And the node "Bad" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Bad"
    When I execute the workflow
    Then the execution succeeds
    And the node "Bad" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: Insert round-trips int, bigint, decimal, float, bit, datetime, datetime2, date, time, uniqueidentifier, nvarchar, varbinary and null
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | microsoftSql  | 1.2         |
      | Insert  | microsoftSql  | 1.2         |
      | Select  | microsoftSql  | 1.2         |
      | Cleanup | microsoftSql  | 1.2         |
    And the node "Setup" has parameters:
      """
      {
        "operation": "executeQuery",
        "query": "CREATE TABLE ms_bdd_types(id INT PRIMARY KEY, big BIGINT, amt DECIMAL(12,2), spd FLOAT, flag BIT, dt DATETIME, dt2 DATETIME2, d DATE, t TIME, guid UNIQUEIDENTIFIER, name NVARCHAR(50), blob VARBINARY(50), nothing NVARCHAR(10))",
        "options": {}
      }
      """
    And the trigger outputs the items:
      """
      [{
        "id": 1,
        "big": 9223372036854775807,
        "amt": 123.45,
        "spd": 3.5,
        "flag": true,
        "dt": "2024-01-15 10:30:00",
        "dt2": "2024-01-15T10:30:00",
        "d": "2024-01-15",
        "t": "10:30:00",
        "guid": "6f9619ff-8b86-d011-b42d-00c04fc964ff",
        "name": "Alice",
        "blob": "AB",
        "nothing": null
      }]
      """
    And the node "Insert" has parameters:
      """
      {"operation": "insert", "table": "ms_bdd_types", "columns": "id,big,amt,spd,flag,dt,dt2,d,t,guid,name,blob,nothing"}
      """
    And the node "Select" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM ms_bdd_types", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_types", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Insert" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Select" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup -> Insert -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs:
      """
      [{"id": 1, "big": 9223372036854775807, "amt": 123.45, "spd": 3.5, "flag": true, "dt": "2024-01-15 10:30:00", "dt2": "2024-01-15T10:30:00", "d": "2024-01-15", "t": "10:30:00", "guid": "6f9619ff-8b86-d011-b42d-00c04fc964ff", "name": "Alice", "blob": "AB", "nothing": null}]
      """
    And the field "id" of item 0 from the node "Select" is 1
    And the field "big" of item 0 from the node "Select" is 9223372036854775807
    And the field "amt" of item 0 from the node "Select" is 123.45
    And the field "spd" of item 0 from the node "Select" is 3.5
    And the field "flag" of item 0 from the node "Select" is true
    And the field "dt" of item 0 from the node "Select" is "2024-01-15T10:30:00.000Z"
    And the field "dt2" of item 0 from the node "Select" is "2024-01-15T10:30:00.000Z"
    And the field "d" of item 0 from the node "Select" is "2024-01-15T00:00:00.000Z"
    And the field "t" of item 0 from the node "Select" is "1970-01-01T10:30:00.000Z"
    And the field "guid" of item 0 from the node "Select" is "6f9619ff-8b86-d011-b42d-00c04fc964ff"
    And the field "name" of item 0 from the node "Select" is "Alice"
    And the field "blob" of item 0 from the node "Select" is "$nonempty"
    And the field "nothing" of item 0 from the node "Select" is null

  Scenario: Insert chunks a large batch across multiple statements once the parameter limit is crossed
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Setup | microsoftSql  | 1.2         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE ms_bdd_chunk(id INT PRIMARY KEY, val NVARCHAR(10))", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MsChunkWrite" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Gen     | code          |             |
      | Insert  | microsoftSql  | 1.2         |
      | Count   | microsoftSql  | 1.2         |
      | Cleanup | microsoftSql  | 1.2         |
    And the node "Gen" runs the JavaScript:
      """
      return Array.from({ length: 1200 }, (_, i) => ({ json: { id: i + 1, val: 'v' + i } }));
      """
    And the node "Insert" has parameters:
      """
      {"operation": "insert", "table": "ms_bdd_chunk", "columns": "id,val"}
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM ms_bdd_chunk", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_chunk", "options": {}}
      """
    And the node "Insert" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Count" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Gen -> Insert -> Count -> Cleanup"
    When I execute the workflow "MsChunkWrite"
    Then the execution succeeds
    And the field "cnt" of item 0 from the node "Count" is 1200
    And the node "Insert" outputs 1200 items

  Scenario: Update changes the matched row (the row's own column supplies the match value)
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | microsoftSql  | 1.2         |
      | Seed    | microsoftSql  | 1.2         |
      | Update  | microsoftSql  | 1.2         |
      | Select  | microsoftSql  | 1.2         |
      | Cleanup | microsoftSql  | 1.2         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE ms_bdd_update(id INT PRIMARY KEY, name NVARCHAR(10))", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO ms_bdd_update VALUES (1,'X')", "options": {}}
      """
    And the node "Update" has parameters:
      """
      {"operation": "update", "table": "ms_bdd_update", "updateKey": "id", "columns": "name"}
      """
    And the trigger outputs the items:
      """
      [{"id": 1, "name": "Y"}]
      """
    And the node "Select" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT * FROM ms_bdd_update", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_update", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Seed" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Update" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Select" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup -> Seed -> Update -> Select -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Update" outputs:
      """
      [{"id": 1, "name": "Y"}]
      """
    And the field "name" of item 0 from the node "Select" is "Y"

  Scenario: Delete removes only the matching rows and reports how many were affected
    Given a workflow with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Setup   | microsoftSql  | 1.2         |
      | Seed    | microsoftSql  | 1.2         |
      | Delete  | microsoftSql  | 1.2         |
      | Count   | microsoftSql  | 1.2         |
      | Cleanup | microsoftSql  | 1.2         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE ms_bdd_delete(id INT PRIMARY KEY)", "options": {}}
      """
    And the node "Seed" has parameters:
      """
      {"operation": "executeQuery", "query": "INSERT INTO ms_bdd_delete VALUES (1),(2),(3)", "options": {}}
      """
    And the node "Delete" has parameters:
      """
      {"operation": "delete", "table": "ms_bdd_delete", "deleteKey": "id"}
      """
    And the trigger outputs the items:
      """
      [{"id": 2}]
      """
    And the node "Count" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT count(*) AS cnt FROM ms_bdd_delete", "options": {}}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_delete", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Seed" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Delete" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Count" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup -> Seed -> Delete -> Count -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs:
      """
      [{"rowsAffected": 1}]
      """
    And the field "cnt" of item 0 from the node "Count" is 2

  # n8n bug, faithfully reproduced (see mssql.rs's module doc comment):
  # insert/update/delete wrap parameter resolution AND every query in one
  # try/catch whose `catch` never recomputes the output from the
  # continueOnFail fallback it assigns -- so a failing insert with
  # continueOnFail succeeds with *zero* output items, not an error item
  # and not the echoed input.
  Scenario: continueOnFail on a failing insert succeeds with zero output items (not an error item)
    Given a workflow with nodes:
      | name    | type          | onError               | typeVersion |
      | Start   | manualTrigger |                        |             |
      | Setup   | microsoftSql  |                        | 1.2         |
      | Insert  | microsoftSql  | continueRegularOutput  | 1.2         |
      | Cleanup | microsoftSql  |                        | 1.2         |
    And the node "Setup" has parameters:
      """
      {"operation": "executeQuery", "query": "CREATE TABLE ms_bdd_ins_fail(id INT PRIMARY KEY, val NVARCHAR(10) NOT NULL)", "options": {}}
      """
    And the trigger outputs the items:
      """
      [{"id": 1}]
      """
    And the node "Insert" has parameters:
      """
      {"operation": "insert", "table": "ms_bdd_ins_fail", "columns": "id"}
      """
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_ins_fail", "options": {}}
      """
    And the node "Setup" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Insert" uses the "microsoftSql" credential "Local MSSQL"
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Setup -> Insert -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs 0 items

    Given a workflow named "MsInsFailCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | microsoftSql  | 1.2         |
    And the node "Cleanup" has parameters:
      """
      {"operation": "executeQuery", "query": "DROP TABLE IF EXISTS ms_bdd_ins_fail", "options": {}}
      """
    And the node "Cleanup" uses the "microsoftSql" credential "Local MSSQL"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MsInsFailCleanup"
    Then the execution succeeds

  Scenario: A connection failure fails clearly and never leaks the password
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | microsoftSql  | 1.2         |
    And the credential "Bad MSSQL Host" of type "microsoftSql" with the data:
      """
      {"server": "127.0.0.1", "port": 1499, "database": "r8r", "user": "sa", "password": "super-secret-password", "tls": false, "connectTimeout": 2000}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1", "options": {}}
      """
    And the node "Query" uses the "microsoftSql" credential "Bad MSSQL Host"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "super-secret-password"

  Scenario: A wrong password fails clearly
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | microsoftSql  | 1.2         |
    And the credential "Bad MSSQL Password" of type "microsoftSql" with the data:
      """
      {"server": "127.0.0.1", "port": 1433, "database": "r8r", "user": "sa", "password": "definitely-wrong", "tls": false}
      """
    And the node "Query" has parameters:
      """
      {"operation": "executeQuery", "query": "SELECT 1", "options": {}}
      """
    And the node "Query" uses the "microsoftSql" credential "Bad MSSQL Password"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
