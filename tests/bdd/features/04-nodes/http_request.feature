@spec-6.6 @phase-1 @node-http
Feature: HTTP Request node (v4)
  Makes outbound HTTP calls per item: methods, query, headers, JSON bodies,
  response parsing, full-response and never-error options, and generic
  credential authentication.

  Background:
    Given a mock HTTP service
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Call  | httpRequest   |
    And the connections "Start -> Call"

  Scenario: A JSON object response becomes one item
    Given the mock service responds to GET "/user" with status 200 and body:
      """
      {"id": 1, "name": "Ada"}
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/user", "options": {}}
      """
    When I execute the workflow
    Then the node "Call" outputs:
      """
      [{"id": 1, "name": "Ada"}]
      """

  Scenario: A JSON array response becomes one item per element
    Given the mock service responds to GET "/users" with status 200 and body:
      """
      [{"id": 1}, {"id": 2}]
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/users", "options": {}}
      """
    When I execute the workflow
    Then the node "Call" outputs:
      """
      [{"id": 1}, {"id": 2}]
      """

  Scenario: One request is made per input item, with expressions per item
    Given the mock service responds to GET "/items" with status 200 and body:
      """
      {"ok": true}
      """
    And the trigger outputs the items:
      """
      [{"q": "red"}, {"q": "blue"}]
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/items", "sendQuery": true, "queryParameters": {"parameters": [{"name": "color", "value": "={{ $json.q }}"}]}, "options": {}}
      """
    When I execute the workflow
    Then the mock service received 2 requests to "/items"
    And the last request to "/items" had the query parameter "color" equal to "blue"

  Scenario: Headers and a JSON body are sent
    Given the mock service responds to POST "/orders" with status 201 and body:
      """
      {"created": true}
      """
    And the trigger outputs the items:
      """
      [{"sku": "A-1", "qty": 2}]
      """
    And the node "Call" has parameters:
      """
      {
        "method": "POST",
        "url": "%{MOCK_URL}/orders",
        "sendHeaders": true,
        "headerParameters": {"parameters": [{"name": "X-Request-Id", "value": "abc-123"}]},
        "sendBody": true,
        "specifyBody": "json",
        "jsonBody": "={\"sku\": \"{{ $json.sku }}\", \"qty\": {{ $json.qty }}}",
        "options": {}
      }
      """
    When I execute the workflow
    Then the node "Call" outputs:
      """
      [{"created": true}]
      """
    And the last request to "/orders" had the header "X-Request-Id" equal to "abc-123"
    And the last request to "/orders" had a JSON body matching:
      """
      {"sku": "A-1", "qty": 2}
      """

  Scenario: Body parameters are sent as JSON fields
    Given the mock service responds to POST "/form" with status 200
    And the node "Call" has parameters:
      """
      {"method": "POST", "url": "%{MOCK_URL}/form", "sendBody": true, "contentType": "json",
       "bodyParameters": {"parameters": [{"name": "a", "value": "1"}, {"name": "b", "value": "two"}]}, "options": {}}
      """
    When I execute the workflow
    Then the last request to "/form" had a JSON body matching:
      """
      {"a": "1", "b": "two"}
      """

  Scenario: The full response includes status code and headers
    Given the mock service responds to GET "/full" with status 200 and body:
      """
      {"v": 1}
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/full", "options": {"response": {"response": {"fullResponse": true}}}}
      """
    When I execute the workflow
    Then the node "Call" outputs items matching:
      """
      [{"statusCode": 200, "headers": {"content-type": "$contains:application/json"}, "body": {"v": 1}}]
      """

  Scenario: An error status fails the node with the status in the message
    Given the mock service responds to GET "/nope" with status 404
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/nope", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Call" failed with an error containing "404"

  Scenario: neverError returns error responses as data
    Given the mock service responds to GET "/nope" with status 404 and body:
      """
      {"error": "not found"}
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/nope", "options": {"response": {"response": {"neverError": true, "fullResponse": true}}}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Call" outputs items matching:
      """
      [{"statusCode": 404, "body": {"error": "not found"}}]
      """

  Scenario: A text response is returned in a data field
    Given the mock service responds to GET "/text" with status 200 and body:
      """
      plain words
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/text", "options": {"response": {"response": {"responseFormat": "text"}}}}
      """
    When I execute the workflow
    Then the node "Call" outputs:
      """
      [{"data": "plain words"}]
      """

  Scenario: A request that exceeds the timeout option fails
    Given the mock service responds to GET "/slow" with status 200 after 3000 ms
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/slow", "options": {"timeout": 500}}
      """
    When I execute the workflow
    Then the execution fails
    And the command finished within 2900 ms

  Scenario: Header auth credentials are applied and never stored in the run data
    Given the credential "Partner API" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-API-Key", "value": "s3cr3t-partner-key"}
      """
    And the mock service responds to GET "/secure" with status 200 and body:
      """
      {"authorised": true}
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/secure", "authentication": "genericCredentialType", "genericAuthType": "httpHeaderAuth", "options": {}}
      """
    And the node "Call" uses the "httpHeaderAuth" credential "Partner API"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/secure" had the header "X-API-Key" equal to "s3cr3t-partner-key"
    And the execution data does not contain "s3cr3t-partner-key"

  Scenario: Basic auth credentials are applied
    Given the credential "Legacy" of type "httpBasicAuth" with the data:
      """
      {"user": "ada", "password": "lovelace"}
      """
    And the mock service responds to GET "/basic" with status 200
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/basic", "authentication": "genericCredentialType", "genericAuthType": "httpBasicAuth", "options": {}}
      """
    And the node "Call" uses the "httpBasicAuth" credential "Legacy"
    When I execute the workflow
    Then the last request to "/basic" had the header "authorization" equal to "Basic YWRhOmxvdmVsYWNl"

  Scenario: Pagination follows pages until a condition is met
    Given the mock service responds to GET "/pages" with status 200 and body:
      """
      {"items": [1], "next": null}
      """
    And the node "Call" has parameters:
      """
      {"url": "%{MOCK_URL}/pages", "options": {"pagination": {"pagination": {
        "paginationMode": "updateAParameterInEachRequest",
        "parameters": {"parameters": [{"type": "qs", "name": "page", "value": "={{ $pageCount + 1 }}"}]},
        "paginationCompleteWhen": "other",
        "completeExpression": "={{ $response.body.next === null }}",
        "limitPagesFetched": true, "maxRequests": 5
      }}}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/pages"
