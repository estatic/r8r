@spec-6.6 @phase-4 @node-airtable
Feature: Airtable node
  Consumes the Airtable REST API (v2/2.2, n8n's `defaultVersion` -- what the
  n8n 2.35.7 editor creates) against the `airtableTokenApi` (Personal
  Access Token) and `airtableOAuth2Api` credentials. Implements the `base`
  resource's getMany/getSchema and the `record` resource's create,
  deleteRecord, get, search, update and upsert operations. Airtable Trigger
  and the `table` resource are out of scope; anything unimplemented fails
  with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "PAT" of type "airtableTokenApi" with the data:
      """
      {"accessToken": "secret_pat_token", "url": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | airtable      |
    And the connections "Start -> Node"

  # ---- record: create -------------------------------------------------------

  Scenario: Creating a record with columns defined below sends typecast and fields
    Given the mock service responds to POST "/app1/tbl1" with status 200 and body:
      """
      {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Widget", "Price": 9.99}}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "create",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {"Name": "Widget", "Price": 9.99}},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had the header "authorization" equal to "Bearer secret_pat_token"
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "fields": {"Name": "Widget", "Price": 9.99}}
      """
    And the node "Node" outputs:
      """
      [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Widget", "Price": 9.99}}]
      """

  Scenario: Creating a record with Auto-Map Input Data ignores the listed fields
    Given the mock service responds to POST "/app1/tbl1" with status 200 and body:
      """
      {"id": "rec2", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Gadget"}}
      """
    And the trigger outputs the items:
      """
      [{"Name": "Gadget", "internalNote": "skip me"}]
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "create",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "autoMapInputData", "value": {}},
        "options": {"ignoreFields": "internalNote"}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "fields": {"Name": "Gadget"}}
      """

  Scenario: Creating a record with typecast coerces a JSON-stringified array field
    Given the mock service responds to POST "/app1/tbl1" with status 200 and body:
      """
      {"id": "rec3", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Tags": ["a", "b"]}}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "create",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {
          "mappingMode": "defineBelow",
          "value": {"Tags": "[\"a\",\"b\"]"},
          "schema": [{"id": "Tags", "type": "array"}]
        },
        "options": {"typecast": true}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": true, "fields": {"Tags": ["a", "b"]}}
      """

  # ---- record: get -----------------------------------------------------------

  Scenario: Getting a record at the default node version keeps fields nested
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 200 and body:
      """
      {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Widget"}}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "get",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "id": "rec1",
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Widget"}}]
      """

  Scenario: Getting a record on node version 2.1 flattens fields into the top level
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 200 and body:
      """
      {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Widget"}}
      """
    And a workflow with nodes:
      | name  | type     | typeVersion |
      | Start | manualTrigger |        |
      | Old   | airtable | 2.1         |
    And the connections "Start -> Old"
    And the node "Old" has parameters:
      """
      {
        "resource": "record",
        "operation": "get",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "id": "rec1",
        "options": {}
      }
      """
    And the node "Old" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Old" outputs:
      """
      [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "Name": "Widget"}]
      """

  Scenario: A table name with spaces is encodeURI-escaped in the request path
    Given the mock service responds to GET "/app1/My%20Table/rec1" with status 200 and body:
      """
      {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "get",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "My Table"},
        "id": "rec1",
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/app1/My%20Table/rec1"

  # ---- record: deleteRecord --------------------------------------------------

  Scenario: Deleting a record returns Airtable's deletion confirmation as-is
    Given the mock service responds to DELETE "/app1/tbl1/rec1" with status 200 and body:
      """
      {"id": "rec1", "deleted": true}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "deleteRecord",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "id": "rec1"
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "rec1", "deleted": true}]
      """

  # ---- record: search ---------------------------------------------------------

  Scenario: Searching with a limit sends filterByFormula, fields, sort, view and maxRecords
    Given the mock service responds to GET "/app1/tbl1" with status 200 and body:
      """
      {"records": [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "A"}}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "search",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "filterByFormula": "NOT({Name} = '')",
        "returnAll": false,
        "limit": 5,
        "sort": {"property": [{"field": "Name", "direction": "asc"}]},
        "options": {"fields": ["Name", "Price"], "view": {"mode": "id", "value": "viwXXX"}}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had the query parameter "filterByFormula" equal to "NOT({Name} = '')"
    And the last request to "/app1/tbl1" had the query parameter "fields[0]" equal to "Name"
    And the last request to "/app1/tbl1" had the query parameter "fields[1]" equal to "Price"
    And the last request to "/app1/tbl1" had the query parameter "sort[0][field]" equal to "Name"
    And the last request to "/app1/tbl1" had the query parameter "sort[0][direction]" equal to "asc"
    And the last request to "/app1/tbl1" had the query parameter "view" equal to "viwXXX"
    And the last request to "/app1/tbl1" had the query parameter "maxRecords" equal to "5"
    And the node "Node" outputs:
      """
      [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "A"}}]
      """

  Scenario: Searching with Return All paginates on offset until it is absent
    Given the mock service responds to GET "/app1/tbl1" in order with:
      """
      [
        {"status": 200, "body": {"records": [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}}], "offset": "offset-2"}},
        {"status": 200, "body": {"records": [{"id": "rec2", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}}]}}
      ]
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "search",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "filterByFormula": "",
        "returnAll": true,
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/app1/tbl1"
    And the 1st request to "/app1/tbl1" had the header "authorization" equal to "Bearer secret_pat_token"
    And the node "Node" outputs:
      """
      [
        {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}},
        {"id": "rec2", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}}
      ]
      """

  # ---- record: update ---------------------------------------------------------

  Scenario: Updating a record matching on the Record ID column
    Given the mock service responds to PATCH "/app1/tbl1" with status 200 and body:
      """
      {"records": [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Renamed"}}]}
      """
    And the trigger outputs the items:
      """
      [{"id": "rec1", "Name": "Renamed"}]
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "update",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "autoMapInputData", "value": {}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "records": [{"id": "rec1", "fields": {"Name": "Renamed"}}]}
      """
    And the node "Node" outputs:
      """
      [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "Renamed"}}]
      """

  Scenario: Updating a record matching on a non-ID column fetches candidates first
    Given the mock service responds to GET "/app1/tbl1" with query parameter "fields[0]" equal to "Email" with status 200 and body:
      """
      {"records": [{"id": "recA", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Email": "a@x.com"}}]}
      """
    And the mock service responds to PATCH "/app1/tbl1" with status 200 and body:
      """
      {"records": [{"id": "recA", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Email": "a@x.com", "Name": "NewName"}}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "update",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {"Email": "a@x.com", "Name": "NewName"}, "matchingColumns": ["Email"]},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "records": [{"id": "recA", "fields": {"Name": "NewName"}}]}
      """

  Scenario: Updating a record with no matching column fails with Airtable's own message
    Given the mock service responds to GET "/app1/tbl1" with query parameter "fields[0]" equal to "Email" with status 200 and body:
      """
      {"records": [{"id": "recA", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Email": "other@x.com"}}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "update",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {"Email": "nope@x.com", "Name": "NewName"}, "matchingColumns": ["Email"]},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Record matching provided keys was not found"

  # ---- record: upsert ----------------------------------------------------------

  Scenario: Upserting a record matching on the Record ID column updates directly
    Given the mock service responds to PATCH "/app1/tbl1" with status 200 and body:
      """
      {"records": [{"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Name": "X"}}]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "upsert",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {"id": "rec1", "Name": "X"}, "matchingColumns": ["id"]},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "records": [{"id": "rec1", "fields": {"Name": "X"}}]}
      """

  Scenario: Upserting a record matching on a non-ID column asks Airtable to performUpsert
    Given the mock service responds to PATCH "/app1/tbl1" with status 200 and body:
      """
      {"records": [{"id": "recNew", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Email": "b@x.com", "Name": "Y"}}], "createdRecords": ["recNew"]}
      """
    And the trigger outputs the items:
      """
      [{"Email": "b@x.com", "Name": "Y"}]
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "record",
        "operation": "upsert",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "autoMapInputData", "value": {}, "matchingColumns": ["Email"]},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had a JSON body matching:
      """
      {"typecast": false, "performUpsert": {"fieldsToMergeOn": ["Email"]}, "records": [{"fields": {"Email": "b@x.com", "Name": "Y"}}]}
      """
    And the node "Node" outputs:
      """
      [{"id": "recNew", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {"Email": "b@x.com", "Name": "Y"}}]
      """

  # ---- base: getMany / getSchema ----------------------------------------------

  Scenario: Getting many bases with Return All paginates on offset
    Given the mock service responds to GET "/meta/bases" in order with:
      """
      [
        {"status": 200, "body": {"bases": [{"id": "app1", "name": "Base One", "permissionLevel": "create"}], "offset": "off-2"}},
        {"status": 200, "body": {"bases": [{"id": "app2", "name": "Base Two", "permissionLevel": "read"}]}}
      ]
      """
    And the node "Node" has parameters:
      """
      {"resource": "base", "operation": "getMany", "returnAll": true, "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/meta/bases"
    And the node "Node" outputs:
      """
      [
        {"id": "app1", "name": "Base One", "permissionLevel": "create"},
        {"id": "app2", "name": "Base Two", "permissionLevel": "read"}
      ]
      """

  Scenario: Getting many bases with a limit and a permission-level filter
    Given the mock service responds to GET "/meta/bases" with status 200 and body:
      """
      {"bases": [
        {"id": "app1", "name": "Base One", "permissionLevel": "create"},
        {"id": "app2", "name": "Base Two", "permissionLevel": "read"},
        {"id": "app3", "name": "Base Three", "permissionLevel": "read"}
      ]}
      """
    And the node "Node" has parameters:
      """
      {"resource": "base", "operation": "getMany", "returnAll": false, "limit": 2, "options": {"permissionLevel": ["read"]}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "app2", "name": "Base Two", "permissionLevel": "read"}]
      """

  Scenario: Getting a base's schema returns its tables
    Given the mock service responds to GET "/meta/bases/app1/tables" with status 200 and body:
      """
      {"tables": [{"id": "tbl1", "name": "Table 1", "primaryFieldId": "fld1"}]}
      """
    And the node "Node" has parameters:
      """
      {"resource": "base", "operation": "getSchema", "base": {"mode": "id", "value": "app1"}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "tbl1", "name": "Table 1", "primaryFieldId": "fld1"}]
      """

  # ---- OAuth2 authentication ---------------------------------------------------

  Scenario: Creating a record using OAuth2 authentication
    Given the credential "OAuth" of type "airtableOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}", "oauthTokenData": {"access_token": "oauth-token-1"}}
      """
    And the mock service responds to POST "/app1/tbl1" with status 200 and body:
      """
      {"id": "rec1", "createdTime": "2024-01-01T00:00:00.000Z", "fields": {}}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "airtableOAuth2Api",
        "resource": "record",
        "operation": "create",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {}},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableOAuth2Api" credential "OAuth"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/app1/tbl1" had the header "authorization" equal to "Bearer oauth-token-1"

  Scenario: An Airtable OAuth2 credential that was never connected fails with a clear message
    Given the credential "Unconnected" of type "airtableOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}"}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "airtableOAuth2Api",
        "resource": "record",
        "operation": "create",
        "base": {"mode": "id", "value": "app1"},
        "table": {"mode": "id", "value": "tbl1"},
        "columns": {"mappingMode": "defineBelow", "value": {}},
        "options": {}
      }
      """
    And the node "Node" uses the "airtableOAuth2Api" credential "Unconnected"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not connected"

  # ---- errors, auth, continueOnFail, and unsupported operations ---------------

  Scenario: A 401 from Airtable maps to n8n's authorization-failed message
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 401 and body:
      """
      {"error": {"type": "AUTHENTICATION_REQUIRED", "message": "Authentication required"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "rec1", "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 403 from Airtable maps to n8n's forbidden message
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 403 and body:
      """
      {"error": {"type": "NOT_AUTHORIZED", "message": "Not authorized"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "rec1", "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Forbidden - perhaps check your credentials?"

  Scenario: A 404 from Airtable maps to n8n's resource-not-found message
    Given the mock service responds to GET "/app1/tbl1/missing" with status 404 and body:
      """
      {"error": "NOT_FOUND"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "missing", "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The resource you are requesting could not be found"

  Scenario: A 422 from Airtable maps to n8n's generic invalid-request message
    Given the mock service responds to POST "/app1/tbl1" with status 422 and body:
      """
      {"error": {"type": "INVALID_VALUE_FOR_COLUMN", "message": "Field value is not valid"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "create", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "columns": {"mappingMode": "defineBelow", "value": {}}, "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Your request is invalid or could not be processed by the service"

  Scenario: A 429 from Airtable maps to n8n's rate-limit message
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 429 and body:
      """
      {"error": {"type": "LIST_RECORDS_RATE_LIMIT_EXCEEDED", "message": "Rate limit exceeded"}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "rec1", "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The service is receiving too many requests from you"

  Scenario: A missing Airtable credential fails with a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "rec1", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: The access token never appears in the execution data
    Given the mock service responds to GET "/app1/tbl1/rec1" with status 401 and body:
      """
      {"error": "AUTHENTICATION_REQUIRED"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "rec1", "options": {}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "secret_pat_token"

  Scenario: continueOnFail turns an Airtable error into an error item instead of failing the run
    Given the mock service responds to GET "/app1/tbl1/missing" with status 404 and body:
      """
      {"error": "NOT_FOUND"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "record", "operation": "get", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}, "id": "missing", "options": {}}
      """
    And the node "Node" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"error": "$contains:The resource you are requesting could not be found"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given the node "Node" has parameters:
      """
      {"resource": "table", "operation": "create"}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on the record resource returns a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "record", "operation": "move", "base": {"mode": "id", "value": "app1"}, "table": {"mode": "id", "value": "tbl1"}}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on the base resource returns a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "base", "operation": "create"}
      """
    And the node "Node" uses the "airtableTokenApi" credential "PAT"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
