@spec-6.6 @phase-4 @node-data-table
Feature: Data table node
  Permanently saves data across workflow executions in a table, backed
  directly by r8r's store (`crate::n8n::data_table`), not an HTTP proxy.
  Faithful to n8n's `DataTable.node.js` (typeVersion 1.1) row operations:
  insert, get (filters, returnAll/limit, orderBy), update, upsert,
  deleteRows (with dryRun), rowExists/rowNotExists.

  Scenario: Insert a row
    Given a data table "Customers" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "insert", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "columns": {"mappingMode": "defineBelow", "value": {"name": "Ada", "age": 30}}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada", "age": 30}]
      """

  Scenario: Get rows with a filter (all conditions)
    Given a data table "People" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}], "rows": [{"name": "Ada", "age": 30}, {"name": "Grace", "age": 40}, {"name": "Ada", "age": 99}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "matchType": "allConditions", "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}, {"keyName": "age", "condition": "gt", "keyValue": 50}]}, "returnAll": true}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 3, "name": "Ada", "age": 99}]
      """

  Scenario: Get with returnAll false respects the limit
    Given a data table "ManyRows" defined as:
      """
      {"columns": [{"name": "n", "type": "number"}], "rows": [{"n": 1}, {"n": 2}, {"n": 3}, {"n": 4}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "returnAll": false, "limit": 2, "orderBy": true, "orderByColumn": "n", "orderByDirection": "ASC"}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "n": 1}, {"id": 2, "n": 2}]
      """

  Scenario: Update rows matching a filter
    Given a data table "ToUpdate" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}], "rows": [{"name": "Ada", "age": 30}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "update", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}]}, "columns": {"mappingMode": "defineBelow", "value": {"age": 31}}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada", "age": 31}]
      """

  Scenario: Upsert inserts when there is no match
    Given a data table "UpsertNoMatch" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "upsert", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Grace"}]}, "columns": {"mappingMode": "defineBelow", "value": {"name": "Grace", "age": 40}}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Grace", "age": 40}]
      """

  Scenario: Upsert updates when there is a match
    Given a data table "UpsertMatch" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}], "rows": [{"name": "Grace", "age": 40}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "upsert", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Grace"}]}, "columns": {"mappingMode": "defineBelow", "value": {"age": 41}}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Grace", "age": 41}]
      """

  Scenario: Delete rows with dry run leaves the data untouched
    Given a data table "ToDelete" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}], "rows": [{"name": "Ada"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "deleteRows", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}]}, "options": {"dryRun": true}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada"}]
      """

  Scenario: Delete rows for real removes them
    Given a data table "ToDeleteReal" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}], "rows": [{"name": "Ada"}, {"name": "Grace"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "deleteRows", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}]}}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada"}]
      """

  Scenario: Row exists routes matching items through
    Given a data table "ExistsTable" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}], "rows": [{"name": "Ada"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "rowExists", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}]}}
      """
    And the connections "Start -> Node"
    And the trigger outputs the items:
      """
      [{"name": "Ada"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"name": "Ada"}]
      """

  Scenario: Row does not exist drops matching items
    Given a data table "NotExistsTable" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}], "rows": [{"name": "Ada"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "rowNotExists", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "filters": {"conditions": [{"keyName": "name", "condition": "eq", "keyValue": "Ada"}]}}
      """
    And the connections "Start -> Node"
    And the trigger outputs the items:
      """
      [{"name": "Ada"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      []
      """

  Scenario: Selecting a data table by name
    Given a data table "ByName" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}], "rows": [{"name": "Ada"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "name", "value": "ByName"}, "returnAll": true}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada"}]
      """

  Scenario: An unknown data table id fails the node
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "id", "value": "doesnotexist123"}, "returnAll": true}
      """
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution fails

  Scenario: continueOnFail turns the error into an error item
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "id", "value": "doesnotexist123"}, "returnAll": true}
      """
    And the node "Node" has the property "onError" set to "continueRegularOutput"
    And the connections "Start -> Node"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"error": "Data table with ID \"doesnotexist123\" not found"}]
      """

  Scenario: Autocreating columns via autoMapInputData
    Given a data table "AutoMap" defined as:
      """
      {"columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | dataTable     |
    And the node "Node" has parameters:
      """
      {"resource": "row", "operation": "insert", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "columns": {"mappingMode": "autoMapInputData"}}
      """
    And the connections "Start -> Node"
    And the trigger outputs the items:
      """
      [{"name": "Ada", "age": 30}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": 1, "name": "Ada", "age": 30}]
      """

  Scenario: Creating a table from the node
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | dataTable     |
    And the node "Create" has parameters:
      """
      {"resource": "table", "operation": "create", "name": "FreshTable", "columns": [{"name": "x", "type": "string"}]}
      """
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the node "Create" outputs items matching:
      """
      [{"name": "FreshTable", "columns": [{"name": "x", "type": "string"}]}]
      """

  Scenario: Listing, renaming, inserting into, clearing and deleting a table from the node
    Given a data table "TableOps" defined as:
      """
      {"columns": [{"name": "x", "type": "string"}], "rows": [{"x": "one"}]}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | List   | dataTable     |
      | Rename | dataTable     |
      | Insert | dataTable     |
      | Clear  | dataTable     |
      | Get    | dataTable     |
      | Delete | dataTable     |
    And the node "List" has parameters:
      """
      {"resource": "table", "operation": "list"}
      """
    And the node "Rename" has parameters:
      """
      {"resource": "table", "operation": "update", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "name": "Renamed"}
      """
    And the node "Insert" has parameters:
      """
      {"resource": "row", "operation": "insert", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "columns": {"mappingMode": "defineBelow", "value": {"x": "two"}}}
      """
    And the node "Clear" has parameters:
      """
      {"resource": "table", "operation": "clear", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}}
      """
    And the node "Get" has parameters:
      """
      {"resource": "row", "operation": "get", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}, "returnAll": true}
      """
    And the node "Delete" has parameters:
      """
      {"resource": "table", "operation": "delete", "dataTableId": {"mode": "id", "value": "%{DATA_TABLE_ID}"}}
      """
    And the connections:
      | from   | to     |
      | Start  | List   |
      | List   | Rename |
      | Rename | Insert |
      | Insert | Clear  |
      | Clear  | Get    |
      | Clear  | Delete |
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "TableOps"}]
      """
    And the node "Rename" outputs items matching:
      """
      [{"name": "Renamed"}]
      """
    And the node "Insert" outputs items matching:
      """
      [{"id": 2, "x": "two"}]
      """
    And the node "Clear" outputs:
      """
      [{"success": true}]
      """
    And the node "Get" outputs:
      """
      []
      """
    And the node "Delete" outputs:
      """
      [{"success": true}]
      """
