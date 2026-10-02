@spec-6.9 @phase-2 @data-tables
Feature: Data Tables REST API
  `/rest/projects/:projectId/data-tables` (plus `/rest/data-tables-global`):
  CRUD on data tables and their columns, and row insert/get/update/upsert/
  delete, scoped to a project. Faithful to n8n 2[35].7's
  `DataTableController`/`DataTableService` for routes, request/response
  shapes, filter conditions (`eq`, `neq`, `like`, `ilike`, `gt`, `gte`,
  `lt`, `lte`) and validation messages.

  Background:
    Given a running r8r server with an owner account
    When I log in as the owner
    And I send a GET request to "/rest/projects"
    Then the response status is 200
    And I remember the response JSON at "data[0].id" as "PROJECT_ID"

  Scenario: Creating a data table with columns
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Customers", "columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}]}
      """
    Then the response status is 200
    And the response JSON at "data.name" is "Customers"
    And the response JSON at "data.columns" has 2 elements
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}"
    Then the response status is 200
    And the response JSON at "data.name" is "Customers"

  Scenario: Table names must be unique within a project
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Dup", "columns": []}
      """
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Dup", "columns": []}
      """
    Then the response status is 409

  Scenario: Listing data tables
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "ListA", "columns": []}
      """
    And I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "ListB", "columns": []}
      """
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables"
    Then the response status is 200
    And the response JSON at "data.data" has 2 elements

  Scenario: Renaming and deleting a data table
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "ToRename", "columns": []}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a PATCH request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}" with body:
      """
      {"name": "Renamed"}
      """
    Then the response status is 200
    And the response JSON at "data.name" is "Renamed"
    When I send a DELETE request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}"
    Then the response status is 200
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}"
    Then the response status is 404

  Scenario: Creating a table with an invalid column name is rejected
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "BadCols", "columns": [{"name": "1bad", "type": "string"}]}
      """
    Then the response status is 400

  Scenario: Adding, renaming, moving and deleting a column
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "ColOps", "columns": [{"name": "first", "type": "string"}]}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns" with body:
      """
      {"name": "second", "type": "number"}
      """
    Then the response status is 200
    And I remember the response JSON at "data.id" as "COLUMN_ID"
    When I send a PATCH request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns/%{COLUMN_ID}/rename" with body:
      """
      {"name": "renamed"}
      """
    Then the response status is 200
    And the response JSON at "data.name" is "renamed"
    When I send a PATCH request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns/%{COLUMN_ID}/move" with body:
      """
      {"targetIndex": 0}
      """
    Then the response status is 200
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns"
    Then the response status is 200
    And the response JSON at "data[0].name" is "renamed"
    When I send a DELETE request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns/%{COLUMN_ID}"
    Then the response status is 200

  Scenario: A duplicate or system column name is rejected
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "ColConflict", "columns": [{"name": "dup", "type": "string"}]}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns" with body:
      """
      {"name": "dup", "type": "number"}
      """
    Then the response status is 409
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/columns" with body:
      """
      {"name": "id", "type": "number"}
      """
    Then the response status is 400

  Scenario: Inserting, getting, updating, upserting and deleting rows
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Rows", "columns": [{"name": "name", "type": "string"}, {"name": "age", "type": "number"}, {"name": "active", "type": "boolean"}]}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/insert" with body:
      """
      {"data": [{"name": "Ada", "age": 30, "active": true}, {"name": "Grace", "age": 40, "active": false}], "returnType": "all"}
      """
    Then the response status is 200
    And the response JSON at "data" has 2 elements

    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows?filter=%7B%22filters%22%3A%5B%7B%22columnName%22%3A%22name%22%2C%22condition%22%3A%22eq%22%2C%22value%22%3A%22Ada%22%7D%5D%7D"
    Then the response status is 200
    And the response JSON at "data.count" is 1
    And the response JSON at "data.data[0].age" is 30

    When I send a PATCH request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows" with body:
      """
      {"filter": {"filters": [{"columnName": "name", "condition": "eq", "value": "Ada"}]}, "data": {"age": 31}, "returnData": true}
      """
    Then the response status is 200
    And the response JSON at "data[0].age" is 31

    When I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/upsert" with body:
      """
      {"filter": {"filters": [{"columnName": "name", "condition": "eq", "value": "Nobody"}]}, "data": {"name": "Nobody", "age": 1}, "returnData": true}
      """
    Then the response status is 200
    And the response JSON at "data[0].name" is "Nobody"

    When I send a DELETE request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows?filter=%7B%22filters%22%3A%5B%7B%22columnName%22%3A%22name%22%2C%22condition%22%3A%22eq%22%2C%22value%22%3A%22Nobody%22%7D%5D%7D&returnData=true"
    Then the response status is 200
    And the response JSON at "data[0].name" is "Nobody"

  Scenario: Row delete requires a filter
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "NoFilterDelete", "columns": []}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a DELETE request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows"
    Then the response status is 400

  Scenario: Filtering rows by a numeric comparison
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "NumFilter", "columns": [{"name": "score", "type": "number"}]}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    And I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/insert" with body:
      """
      {"data": [{"score": 1}, {"score": 5}, {"score": 9}], "returnType": "count"}
      """
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows?filter=%7B%22filters%22%3A%5B%7B%22columnName%22%3A%22score%22%2C%22condition%22%3A%22gt%22%2C%22value%22%3A4%7D%5D%7D"
    Then the response status is 200
    And the response JSON at "data.count" is 2

  Scenario: Filtering by an unknown column is rejected
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "UnknownCol", "columns": []}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows?filter=%7B%22filters%22%3A%5B%7B%22columnName%22%3A%22nope%22%2C%22condition%22%3A%22eq%22%2C%22value%22%3A1%7D%5D%7D"
    Then the response status is 400

  Scenario: Pagination with skip and take
    Given I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Paged", "columns": [{"name": "n", "type": "number"}]}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    And I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/insert" with body:
      """
      {"data": [{"n": 1}, {"n": 2}, {"n": 3}], "returnType": "count"}
      """
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}/rows?take=1&skip=1&sortBy=n:asc"
    Then the response status is 200
    And the response JSON at "data.count" is 3
    And the response JSON at "data.data" has 1 element
    And the response JSON at "data.data[0].n" is 2

  Scenario: Unknown data table id
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/doesnotexist123"
    Then the response status is 404

  Scenario: The editor API requires a session
    Given I am not authenticated
    When I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables"
    Then the response status is 401

  Scenario: A user outside the project cannot see its data tables
    Given a member user "outsider@example.com"
    And I send a POST request to "/rest/projects/%{PROJECT_ID}/data-tables" with body:
      """
      {"name": "Private", "columns": []}
      """
    And I remember the response JSON at "data.id" as "TABLE_ID"
    When I am logged in as "outsider@example.com"
    And I send a GET request to "/rest/projects/%{PROJECT_ID}/data-tables/%{TABLE_ID}"
    Then the response status is 404
