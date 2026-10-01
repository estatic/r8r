@spec-6.6 @phase-4 @node-mongodb @requires-mongodb
Feature: MongoDB node
  aggregate, delete (deleteMany), find, findOneAndReplace,
  findOneAndUpdate, insert and update against a real MongoDB 7 (Docker
  `r8r-bdd-mongo`, 127.0.0.1:27017, no auth; opt in with
  R8R_BDD_INCLUDE=requires-mongodb). Each scenario uses its own uniquely
  named collection and drops it again via the node under test itself (a
  "delete" or `aggregate`+`$out`-free cleanup step), so scenarios stay
  independent under parallel execution.

  Background:
    Given the credential "Local MongoDB" of type "mongoDb" with the data:
      """
      {"configurationType": "values", "host": "127.0.0.1", "port": 27017, "database": "r8r_bdd", "user": "", "password": "", "tls": false}
      """

  Scenario: Insert creates documents and returns them with the generated id
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Alice", "age": 30}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_insert", "fields": "name,age", "options": {}}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert"
    When I execute the workflow
    Then the execution succeeds
    And the node "Insert" outputs 1 items
    And the field "name" of item 0 from the node "Insert" is "Alice"
    And the field "age" of item 0 from the node "Insert" is 30
    And the field "id" of item 0 from the node "Insert" is "$nonempty"

  Scenario: Find returns the documents matching the query
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Bob", "age": 25}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_find", "fields": "name,age", "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_find", "query": "{\"name\": \"Bob\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert -> Find"
    When I execute the workflow
    Then the execution succeeds
    And the node "Find" outputs 1 items
    And the field "name" of item 0 from the node "Find" is "Bob"
    And the field "age" of item 0 from the node "Find" is 25
    And the field "_id" of item 0 from the node "Find" is "$nonempty"

  Scenario: Find by ObjectId via a plain _id string
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
      | Delete | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Carol"}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_oid", "fields": "name", "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_oid", "query": "={{ '{\"_id\": \"' + $json.id + '\"}' }}", "queryParameters": "[]", "options": {}}
      """
    And the node "Delete" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_oid", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the node "Delete" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert -> Find -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the field "name" of item 0 from the node "Find" is "Carol"
    And the field "deletedCount" of item 0 from the node "Delete" is 1

  Scenario: Find by ObjectId via EJSON $oid
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
      | Delete | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Dave"}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_ejson", "fields": "name", "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_ejson", "query": "={{ '{\"_id\": {\"$oid\": \"' + $json.id + '\"}}' }}", "queryParameters": "[]", "options": {}}
      """
    And the node "Delete" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_ejson", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the node "Delete" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert -> Find -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the field "name" of item 0 from the node "Find" is "Dave"

  Scenario: Find applies sort, limit, skip and projection
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"n": 1, "score": 10}, {"n": 2, "score": 30}, {"n": 3, "score": 20}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_findopts", "fields": "n,score", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbFindOptsQuery" with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Find  | mongoDb       | 1.4         |
    And the node "Find" has parameters:
      """
      {
        "resource": "document",
        "operation": "find",
        "collection": "mdb_bdd_findopts",
        "query": "{}",
        "queryParameters": "[]",
        "options": {"sort": "{\"score\": -1}", "limit": 2, "skip": 1, "projection": "{\"_id\": 0, \"n\": 1, \"score\": 1}"}
      }
      """
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Find"
    When I execute the workflow "MdbFindOptsQuery"
    Then the execution succeeds
    And the node "Find" outputs:
      """
      [{"n": 3, "score": 20}, {"n": 1, "score": 10}]
      """

    Given a workflow named "MdbFindOptsCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_findopts", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbFindOptsCleanup"
    Then the execution succeeds

  Scenario: Aggregate runs a pipeline and uses query parameters for dynamic values
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"cat": "a", "amount": 5}, {"cat": "a", "amount": 7}, {"cat": "b", "amount": 100}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_agg", "fields": "cat,amount", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbAggQuery" with nodes:
      | name      | type          | typeVersion |
      | Start     | manualTrigger |             |
      | Aggregate | mongoDb       | 1.4         |
    And the node "Aggregate" has parameters:
      """
      {
        "resource": "document",
        "operation": "aggregate",
        "collection": "mdb_bdd_agg",
        "query": "[{\"$match\": {\"cat\": \"$1\"}}, {\"$group\": {\"_id\": \"$cat\", \"total\": {\"$sum\": \"$amount\"}}}]",
        "queryParameters": "[\"a\"]"
      }
      """
    And the node "Aggregate" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Aggregate"
    When I execute the workflow "MdbAggQuery"
    Then the execution succeeds
    And the field "total" of item 0 from the node "Aggregate" is 12

    Given a workflow named "MdbAggCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_agg", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbAggCleanup"
    Then the execution succeeds

  Scenario: Update sets fields on the document matched by updateKey
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "x1", "qty": 1}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_update", "fields": "sku,qty", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbUpdateRun" with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Update | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "x1", "qty": 99}]
      """
    And the node "Update" has parameters:
      """
      {"resource": "document", "operation": "update", "collection": "mdb_bdd_update", "updateKey": "sku", "fields": "sku,qty", "upsert": false, "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_update", "query": "{\"sku\": \"x1\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Update" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Update -> Find"
    When I execute the workflow "MdbUpdateRun"
    Then the execution succeeds
    And the field "qty" of item 0 from the node "Find" is 99

    Given a workflow named "MdbUpdateCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_update", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbUpdateCleanup"
    Then the execution succeeds

  Scenario: Update with upsert inserts when no document matches
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Update | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "new-sku", "qty": 5}]
      """
    And the node "Update" has parameters:
      """
      {"resource": "document", "operation": "update", "collection": "mdb_bdd_upsert", "updateKey": "sku", "fields": "sku,qty", "upsert": true, "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_upsert", "query": "{\"sku\": \"new-sku\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Update" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Update -> Find"
    When I execute the workflow
    Then the execution succeeds
    And the field "qty" of item 0 from the node "Find" is 5

    Given a workflow named "MdbUpsertCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_upsert", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbUpsertCleanup"
    Then the execution succeeds

  Scenario: findOneAndReplace replaces the whole document
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "r1", "qty": 1, "color": "red"}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_replace", "fields": "sku,qty,color", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbReplaceRun" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Replace | mongoDb       | 1.4         |
      | Find    | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "r1", "qty": 2}]
      """
    And the node "Replace" has parameters:
      """
      {"resource": "document", "operation": "findOneAndReplace", "collection": "mdb_bdd_replace", "updateKey": "sku", "fields": "sku,qty", "upsert": false, "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_replace", "query": "{\"sku\": \"r1\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Replace" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Replace -> Find"
    When I execute the workflow "MdbReplaceRun"
    Then the execution succeeds
    And the field "qty" of item 0 from the node "Find" is 2
    And the field "color" of item 0 from the node "Find" is null

    Given a workflow named "MdbReplaceCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_replace", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbReplaceCleanup"
    Then the execution succeeds

  Scenario: findOneAndUpdate merges fields with $set, keeping untouched fields
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "u1", "qty": 1, "color": "blue"}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_fou", "fields": "sku,qty,color", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbFouRun" with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Update | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"sku": "u1", "qty": 9}]
      """
    And the node "Update" has parameters:
      """
      {"resource": "document", "operation": "findOneAndUpdate", "collection": "mdb_bdd_fou", "updateKey": "sku", "fields": "sku,qty", "upsert": false, "options": {}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_fou", "query": "{\"sku\": \"u1\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Update" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Update -> Find"
    When I execute the workflow "MdbFouRun"
    Then the execution succeeds
    And the field "qty" of item 0 from the node "Find" is 9
    And the field "color" of item 0 from the node "Find" is "blue"

    Given a workflow named "MdbFouCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_fou", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbFouCleanup"
    Then the execution succeeds

  Scenario: Insert with dateFields converts a string into a real Mongo date
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
      | Query  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Erin", "joined": "2024-01-15T10:30:00.000Z"}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_dates", "fields": "name,joined", "options": {"dateFields": "joined"}}
      """
    And the node "Query" has parameters:
      """
      {
        "resource": "document",
        "operation": "aggregate",
        "collection": "mdb_bdd_dates",
        "query": "[{\"$match\": {\"name\": \"Erin\"}}, {\"$project\": {\"name\": 1, \"isDate\": {\"$eq\": [{\"$type\": \"$joined\"}, \"date\"]}}}]",
        "queryParameters": "[]"
      }
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the node "Query" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert -> Query"
    When I execute the workflow
    Then the execution succeeds
    And the field "isDate" of item 0 from the node "Query" is true

    Given a workflow named "MdbDatesCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_dates", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbDatesCleanup"
    Then the execution succeeds

  Scenario: Insert with useDotNotation builds a nested document from a dotted field name
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
      | Find   | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Frank", "address.city": "Berlin", "address.zip": "10115"}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_dotnotation", "fields": "name,address.city,address.zip", "options": {"useDotNotation": true}}
      """
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_dotnotation", "query": "{\"name\": \"Frank\"}", "queryParameters": "[]", "options": {}}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB"
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Insert -> Find"
    When I execute the workflow
    Then the execution succeeds
    And the field "address.city" of item 0 from the node "Find" is "Berlin"
    And the field "address.zip" of item 0 from the node "Find" is "10115"

    Given a workflow named "MdbDotCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_dotnotation", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbDotCleanup"
    Then the execution succeeds

  Scenario: Delete removes only the matching documents
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Seed  | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"kind": "keep"}, {"kind": "drop"}, {"kind": "drop"}]
      """
    And the node "Seed" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_delete", "fields": "kind", "options": {}}
      """
    And the node "Seed" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Seed"
    When I execute the workflow
    Then the execution succeeds

    Given a workflow named "MdbDeleteRun" with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Delete | mongoDb       | 1.4         |
    And the node "Delete" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_delete", "query": "{\"kind\": \"drop\"}", "queryParameters": "[]"}
      """
    And the node "Delete" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Delete"
    When I execute the workflow "MdbDeleteRun"
    Then the execution succeeds
    And the field "deletedCount" of item 0 from the node "Delete" is 2

    Given a workflow named "MdbDeleteCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_delete", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbDeleteCleanup"
    Then the execution succeeds

  Scenario: An invalid JSON query fails clearly
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Find  | mongoDb       | 1.4         |
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_badjson", "query": "{not valid json", "queryParameters": "[]", "options": {}}
      """
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Find"
    When I execute the workflow
    Then the execution fails
    And the node "Find" failed with an error containing "Invalid JSON"

  Scenario: continueOnFail turns a failing query into an error item instead of failing the node
    Given a workflow with nodes:
      | name  | type          | onError               | typeVersion |
      | Start | manualTrigger |                        |             |
      | Find  | mongoDb       | continueRegularOutput  | 1.4         |
    And the node "Find" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_cof", "query": "{not valid json", "queryParameters": "[]", "options": {}}
      """
    And the node "Find" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Find"
    When I execute the workflow
    Then the execution succeeds
    And the node "Find" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  Scenario: Connection string configurationType works as an alternative to values
    Given the credential "Local MongoDB Conn String" of type "mongoDb" with the data:
      """
      {"configurationType": "connectionString", "connectionString": "mongodb://127.0.0.1:27017", "database": "r8r_bdd"}
      """
    And a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Insert | mongoDb       | 1.4         |
    And the trigger outputs the items:
      """
      [{"name": "Grace"}]
      """
    And the node "Insert" has parameters:
      """
      {"resource": "document", "operation": "insert", "collection": "mdb_bdd_connstr", "fields": "name", "options": {}}
      """
    And the node "Insert" uses the "mongoDb" credential "Local MongoDB Conn String"
    And the connections "Start -> Insert"
    When I execute the workflow
    Then the execution succeeds
    And the field "name" of item 0 from the node "Insert" is "Grace"

    Given a workflow named "MdbConnStrCleanup" with nodes:
      | name    | type          | typeVersion |
      | Start   | manualTrigger |             |
      | Cleanup | mongoDb       | 1.4         |
    And the node "Cleanup" has parameters:
      """
      {"resource": "document", "operation": "delete", "collection": "mdb_bdd_connstr", "query": "{}", "queryParameters": "[]"}
      """
    And the node "Cleanup" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Cleanup"
    When I execute the workflow "MdbConnStrCleanup"
    Then the execution succeeds

  Scenario: A wrong host fails clearly and never leaks the password
    Given a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | mongoDb       | 1.4         |
    And the credential "Bad MongoDB Host" of type "mongoDb" with the data:
      """
      {"configurationType": "values", "host": "127.0.0.1", "port": 27099, "database": "r8r_bdd", "user": "someuser", "password": "super-secret-password", "tls": false}
      """
    And the node "Query" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_badhost", "query": "{}", "queryParameters": "[]", "options": {}}
      """
    And the node "Query" uses the "mongoDb" credential "Bad MongoDB Host"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "super-secret-password"

  Scenario: Missing connection string with configurationType connectionString fails clearly
    Given the credential "Blank MongoDB Conn String" of type "mongoDb" with the data:
      """
      {"configurationType": "connectionString", "connectionString": "", "database": "r8r_bdd"}
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Query | mongoDb       | 1.4         |
    And the node "Query" has parameters:
      """
      {"resource": "document", "operation": "find", "collection": "mdb_bdd_blankcs", "query": "{}", "queryParameters": "[]", "options": {}}
      """
    And the node "Query" uses the "mongoDb" credential "Blank MongoDB Conn String"
    And the connections "Start -> Query"
    When I execute the workflow
    Then the execution fails
    And the node "Query" failed with an error containing "connection string"

  Scenario: createSearchIndex against a non-Atlas server fails with the server's own error
    Given a workflow with nodes:
      | name   | type          | typeVersion |
      | Start  | manualTrigger |             |
      | Create | mongoDb       | 1.4         |
    And the node "Create" has parameters:
      """
      {"resource": "searchIndexes", "operation": "createSearchIndex", "collection": "mdb_bdd_searchidx", "indexNameRequired": "my_index", "indexType": "search", "indexDefinition": "{\"mappings\": {\"dynamic\": true}}"}
      """
    And the node "Create" uses the "mongoDb" credential "Local MongoDB"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution fails
