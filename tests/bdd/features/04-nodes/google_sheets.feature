@spec-6.6 @phase-4 @node-google-sheets
Feature: Google Sheets node
  Consumes the Google Sheets v4 API (node v4.5/4.6/4.7, as the n8n 2.35.7
  editor creates it) against the `googleApi` (service-account JWT) and
  `googleSheetsOAuth2Api` credentials. Implements the `sheet` resource's
  append, appendOrUpdate, clear, create, delete, read, remove and update
  operations, plus the `spreadsheet` resource's create and
  deleteSpreadsheet. Anything else fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "Service Account" of type "googleApi" with the data:
      """
      {
        "email": "svc@example.iam.gserviceaccount.com",
        "privateKey": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCgHQNrUetQxEOs\nNfsDt8iMM4/YRAkvAXJQNN7J03CZrB5qg/4NE9yhLA/Iy+VMX+5pBrxoXn5ScTMD\ns0g1qtRR5D2tOLsTjKC+m7sOvVzRh4vJWAXAmaWxfrCg59M1AkYvbIDHs3D5hnTq\nG2ANmA5C9l5zYCeQRH3Niama6ZcI+aaOlwGWklVfTCOBlbIjhqGTr1XYo+vEu+d7\nCZRUB1AR8AphEUQKKxVzQCftnKQAt6lIYWj0Ivt57JtYeKtmvyM1a/nIm/7VH0UH\nctZQljtVKShYF/Mx0bgugJRzaXwFdoKHHddP+eZOUlxmrYdNM4LmU+KofnbMwUpN\nXnRq7OWlAgMBAAECggEAI7NMaZ/GqQfV8f5IRaQzKIWvr2A+Lvey2izrtxM44l7l\nGklrelWBJoO5UkOaUTz7nvnkRR46qt24Kv1M3sCEHm1WjjrdxaZfyhhVrVvuJ/8B\n3WdNwwvPTOQLdJk0N/fVl+nozf5V9KE1DOUFNgq/NVGTLkUUlT+cUFm/Uj3+0f5k\nCFcD/K8SnaukzCRXhXD82j+/6dtAaPyjkEHNKHYofwzvQPtYTdpwYS0Dts9hXmyb\n3g7DU2ewyPevWZSoZ4n9wKTpQ6GamGTdJEtu40VIJGIxQ/yBQYz7LXm1X/EriDMJ\nex4hoTC18Gvtjyo7uFA+nINcwqpG/V0W7D3+CNSdaQKBgQDNVXwE7rn9DOhlzHMm\nFkcHe7p3tgcCts/NC775Kyw+So9+qFRQRdQ1Hminx5w4a3+fR6eVGMf57Xl2AkGy\nat8F6KXn3iXT4qml/J/arkgGsd9YrUBAtN173owdEoNJP1wE4ho7Q58Y+U+zQ51D\n/SZYpRhjiAK+1eJpJWJRfXzGiQKBgQDHnwnfu5Vh4Of1BMTuhVK6O3lxolYYF8w3\nx+H65EN2EjXlG/EYw3RsG9QHhTqF6JVM6vyqFZjRfcuz9DET5fcn0DKvR3D7iVeU\ne/2pCHE+uvEZ9Su0gHZiXT68c/1c+9C7nXqexW16uTm5G7WOZMpEEaAIdRp/toEu\n5AWKDS8fPQKBgDxYvFtCwhyx93c7sDfoYjW70mCueb79dXMg2Z6nZphkF2o1FJqG\n+0glSMLOsoYOafKo/4KdRuCYP5NENIS4ThWRe3j63Ak623syFNUTVY3KJwcL3A9o\nWJO4I1vD/hu/6E5zGRyD0jVnyFm6LHU36FYzJ0jRR2VIvQMD/rJOfCZpAoGBAKnf\nIu3rmXGTjJCrIFLBzeaBGhWjSZRzG+wUArAYc3gUgxyWrJKgMYCWJdbIf0bY58Ru\n69hpTIRpgmF+2gzO04Zj293g87p547eN1Ax2DLiPKQEn66tM7nFCXFLOebsY50Xg\n+yoFY+bdnMtzUwr7pkxKm17XGFe6HTCkBjq2gXUZAoGBAJ8KzbfnHGmmpimolnL+\nTPXNJP6QSwQi92UgIIbh8/BsFyW+5X/3cHhmFo5IlHlOktbT+zru6jv3EG6omqNQ\nWRlFVR6p6VwWjqzTZbR+dZUbRpSAk3jFZ+HIIkhfYpupMYySUf6UpQL10hQWrlYT\n87vTIKToXQtdQtTCFKSYkryf\n-----END PRIVATE KEY-----",
        "url": "%{MOCK_URL}",
        "tokenUrl": "%{MOCK_URL}/oauth2/token"
      }
      """
    And the mock service responds to POST "/oauth2/token" with status 200 and body:
      """
      {"access_token": "svc-token", "expires_in": 3600}
      """
    And the mock service responds to GET "/v4/spreadsheets/SS1" with status 200 and body:
      """
      {"sheets": [{"properties": {"sheetId": 0, "title": "Sheet1"}}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | googleSheets  |
    And the connections "Start -> Node"

  Scenario: Append Row (Map Each Column Below) hits the right range and authenticates as the service account
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["name", "age"], ["Bob", "25"]]}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {"spreadsheetId": "SS1", "replies": [{}]}
      """
    And the mock service responds to PUT "/v4/spreadsheets/SS1/values/Sheet1!3:3" with status 200 and body:
      """
      {"updatedRows": 1}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "append",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "defineBelow", "value": {"name": "Ada", "age": "30"}},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1" had the header "authorization" equal to "Bearer svc-token"
    And the last request to "/v4/spreadsheets/SS1/values/Sheet1!3:3" had a JSON body matching:
      """
      {"range": "Sheet1!3:3", "values": [["Ada", "30"]]}
      """
    And the node "Node" outputs:
      """
      [{"name": "Ada", "age": "30"}]
      """

  Scenario: Append Row (Auto-Map Input Data) writes a header row into an empty sheet
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {}
      """
    And the mock service responds to PUT "/v4/spreadsheets/SS1/values/Sheet1!1:1" with status 200 and body:
      """
      {}
      """
    And the mock service responds to PUT "/v4/spreadsheets/SS1/values/Sheet1!2:2" with status 200 and body:
      """
      {}
      """
    And the trigger outputs the items:
      """
      [{"name": "Carl", "age": "40"}]
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "append",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "autoMapInputData", "value": {}},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/v4/spreadsheets/SS1/values/Sheet1!1:1"
    And the last request to "/v4/spreadsheets/SS1/values/Sheet1!1:1" had a JSON body matching:
      """
      {"range": "Sheet1!1:1", "values": [["name", "age"]]}
      """
    And the last request to "/v4/spreadsheets/SS1/values/Sheet1!2:2" had a JSON body matching:
      """
      {"range": "Sheet1!2:2", "values": [["Carl", "40"]]}
      """
    And the node "Node" outputs:
      """
      [{"name": "Carl", "age": "40"}]
      """

  Scenario: Append or Update Row updates an existing row instead of appending
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["id", "name"], ["1", "OldName"]]}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1/values:batchUpdate" with status 200 and body:
      """
      {"totalUpdatedRows": 1}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "appendOrUpdate",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": "1", "name": "NewName"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/v4/spreadsheets/SS1/values:batchUpdate"
    And the last request to "/v4/spreadsheets/SS1/values:batchUpdate" had a JSON body matching:
      """
      {"data": [{"range": "Sheet1!B2", "values": [["NewName"]]}], "valueInputOption": "USER_ENTERED"}
      """
    And the node "Node" outputs:
      """
      [{"id": "1", "name": "NewName"}]
      """

  Scenario: Append or Update Row appends a new row when no match is found
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["id", "name"], ["1", "OldName"]]}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {}
      """
    And the mock service responds to PUT "/v4/spreadsheets/SS1/values/Sheet1!3:3" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "appendOrUpdate",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": "2", "name": "NewPerson"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1/values/Sheet1!3:3" had a JSON body matching:
      """
      {"range": "Sheet1!3:3", "values": [["2", "NewPerson"]]}
      """

  Scenario: Update Row updates a matching row
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["id", "name"], ["7", "Old"]]}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1/values:batchUpdate" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "update",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": "7", "name": "Updated"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1/values:batchUpdate" had a JSON body matching:
      """
      {"data": [{"range": "Sheet1!B2", "values": [["Updated"]]}]}
      """

  Scenario: Update Row without a matching column fails with a clear message
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["id", "name"]]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "update",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": "1"}, "matchingColumns": []},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "matchingColumns` is required"

  Scenario: Clear wipes the whole sheet
    Given the mock service responds to POST "/v4/spreadsheets/SS1/values/'Sheet1':clear" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "clear",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "clear": "wholeSheet",
        "keepFirstRow": false
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/v4/spreadsheets/SS1/values/'Sheet1':clear"

  Scenario: Create Sheet adds a new tab
    Given the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {"replies": [{"addSheet": {"properties": {"sheetId": 5, "title": "NewSheet", "index": 1}}}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "create",
        "documentId": {"mode": "id", "value": "SS1"},
        "title": "NewSheet",
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"sheetId": 5, "title": "NewSheet", "index": 1}]
      """

  Scenario: Delete Rows removes a dimension range
    Given the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "delete",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "toDelete": "rows",
        "startIndex": 2,
        "numberToDelete": 3
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1:batchUpdate" had a JSON body matching:
      """
      {"requests": [{"deleteDimension": {"range": {"sheetId": 0, "dimension": "ROWS", "startIndex": 1, "endIndex": 4}}}]}
      """
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  Scenario: Delete Columns removes a dimension range
    Given the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "delete",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "toDelete": "columns",
        "startIndex": "B",
        "numberToDelete": 1
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1:batchUpdate" had a JSON body matching:
      """
      {"requests": [{"deleteDimension": {"range": {"sheetId": 0, "dimension": "COLUMNS", "startIndex": 1, "endIndex": 2}}}]}
      """

  Scenario: Delete (Sheet) permanently removes a tab
    Given the mock service responds to POST "/v4/spreadsheets/SS1:batchUpdate" with status 200 and body:
      """
      {"spreadsheetId": "SS1", "replies": [{}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "remove",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1:batchUpdate" had a JSON body matching:
      """
      {"requests": [{"deleteSheet": {"sheetId": 0}}]}
      """
    And the node "Node" outputs:
      """
      [{"spreadsheetId": "SS1"}]
      """

  Scenario: Get Row(s) with no filters includes the row_number column
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["name", "age"], ["Ann", "20"], ["Bo", "21"]]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "read",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "filtersUI": {"values": []},
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"row_number": 2, "name": "Ann", "age": "20"}, {"row_number": 3, "name": "Bo", "age": "21"}]
      """

  Scenario: Get Row(s) with a filter returns only the matching row
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["name", "age"], ["Ann", "20"], ["Bo", "21"]]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "read",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "filtersUI": {"values": [{"lookupColumn": "name", "lookupValue": "Bo"}]},
        "combineFilters": "AND",
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"row_number": 3, "name": "Bo", "age": "21"}]
      """

  Scenario: Get Row(s) with a filter that matches nothing returns no items
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["name", "age"], ["Ann", "20"]]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "read",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "filtersUI": {"values": [{"lookupColumn": "name", "lookupValue": "Zed"}]},
        "combineFilters": "OR",
        "options": {}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      []
      """

  Scenario: Get Row(s) honours a custom header row
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 200 and body:
      """
      {"values": [["ignore", "this"], ["id", "val"], ["1", "Alpha"], ["2", "Beta"]]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "sheet",
        "operation": "read",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "filtersUI": {"values": []},
        "options": {"dataLocationOnSheet": {"values": {"rangeDefinition": "specifyRange", "headerRow": 2, "firstDataRow": 3}}}
      }
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"row_number": 3, "id": "1", "val": "Alpha"}, {"row_number": 4, "id": "2", "val": "Beta"}]
      """

  Scenario: Spreadsheet Create makes a new spreadsheet
    Given the mock service responds to POST "/v4/spreadsheets" with status 200 and body:
      """
      {"spreadsheetId": "NEW1", "properties": {"title": "My New Sheet"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "spreadsheet", "operation": "create", "title": "My New Sheet", "sheetsUi": {}, "options": {}}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"spreadsheetId": "NEW1", "properties": {"title": "My New Sheet"}}]
      """

  Scenario: Spreadsheet Delete removes the Drive file
    Given the mock service responds to DELETE "/drive/v3/files/DOC1" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {"resource": "spreadsheet", "operation": "deleteSpreadsheet", "documentId": {"mode": "id", "value": "DOC1"}}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  Scenario: A missing Google credential fails with a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "sheet", "operation": "read", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: A 403 from Google Sheets becomes a clear error with a permissions hint
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 403 and body:
      """
      {"error": {"code": 403, "message": "The caller does not have permission", "status": "PERMISSION_DENIED"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "sheet", "operation": "read", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}, "filtersUI": {"values": []}, "options": {}}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have permission"

  Scenario: A 404 from Google Sheets becomes a clear error and the token never leaks
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 404 and body:
      """
      {"error": {"code": 404, "message": "Requested entity was not found."}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "sheet", "operation": "read", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}, "filtersUI": {"values": []}, "options": {}}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "was not found"
    And the execution data does not contain "svc-token"

  Scenario: continueOnFail turns a Google Sheets error into an error item instead of failing the run
    Given the mock service responds to GET "/v4/spreadsheets/SS1/values/'Sheet1'" with status 500
    And the node "Node" has parameters:
      """
      {"resource": "sheet", "operation": "read", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}, "filtersUI": {"values": []}, "options": {}}
      """
    And the node "Node" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"error": "$contains:status code 500"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given the node "Node" has parameters:
      """
      {"resource": "userGroup", "operation": "getAll"}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "sheet", "operation": "copy", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}}
      """
    And the node "Node" uses the "googleApi" credential "Service Account"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: Clear authenticates with a connected OAuth2 credential
    Given the credential "OAuth Sheets" of type "googleSheetsOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}", "oauthTokenData": {"access_token": "sheets-oauth-token"}}
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1/values/'Sheet1':clear" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "oAuth2",
        "resource": "sheet",
        "operation": "clear",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "clear": "wholeSheet"
      }
      """
    And the node "Node" uses the "googleSheetsOAuth2Api" credential "OAuth Sheets"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v4/spreadsheets/SS1/values/'Sheet1':clear" had the header "authorization" equal to "Bearer sheets-oauth-token"

  Scenario: A Google Sheets OAuth2 credential that was never connected fails with a clear message
    Given the credential "Unconnected" of type "googleSheetsOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "sheet", "operation": "clear", "documentId": {"mode": "id", "value": "SS1"}, "sheetName": {"mode": "id", "value": "0"}, "clear": "wholeSheet"}
      """
    And the node "Node" uses the "googleSheetsOAuth2Api" credential "Unconnected"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not connected"

  Scenario: A 401 triggers one OAuth2 token refresh and a retry
    Given the credential "OAuth Sheets" of type "googleSheetsOAuth2Api" with the data:
      """
      {
        "url": "%{MOCK_URL}",
        "accessTokenUrl": "%{MOCK_URL}/oauth2/refresh",
        "clientId": "client-1",
        "clientSecret": "secret-1",
        "oauthTokenData": {"access_token": "expired-token", "refresh_token": "refresh-1"}
      }
      """
    And the mock service responds to POST "/v4/spreadsheets/SS1/values/'Sheet1':clear" with status 401 the first 1 times
    And the mock service responds to POST "/v4/spreadsheets/SS1/values/'Sheet1':clear" with status 200 and body:
      """
      {}
      """
    And the mock service responds to POST "/oauth2/refresh" with status 200 and body:
      """
      {"access_token": "fresh-token", "refresh_token": "refresh-2"}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "oAuth2",
        "resource": "sheet",
        "operation": "clear",
        "documentId": {"mode": "id", "value": "SS1"},
        "sheetName": {"mode": "id", "value": "0"},
        "clear": "wholeSheet"
      }
      """
    And the node "Node" uses the "googleSheetsOAuth2Api" credential "OAuth Sheets"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/v4/spreadsheets/SS1/values/'Sheet1':clear"
    And the mock service received 1 request to "/oauth2/refresh"
    And the last request to "/v4/spreadsheets/SS1/values/'Sheet1':clear" had the header "authorization" equal to "Bearer fresh-token"
