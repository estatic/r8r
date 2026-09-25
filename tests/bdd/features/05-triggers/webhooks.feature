@spec-6.3 @spec-4.3 @phase-2
Feature: Webhook trigger
  Active workflows answer on `/webhook/:path`, the same URL n8n uses, so
  callers need no changes after migration. Test URLs under `/webhook-test/`
  work only while the editor is listening.

  Background:
    Given a running r8r server with an owner and an API key
    And a workflow named "Orders" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "POST", "path": "orders", "responseMode": "lastNode", "options": {}}     |
      | Echo    | set     |                                                                                        |
    And the node "Echo" sets the fields:
      """
      {
        "sku": "={{ $json.body.sku }}",
        "source": "={{ $json.query.source }}",
        "agent": "={{ $json.headers['user-agent'] }}",
        "mode": "={{ $json.executionMode }}",
        "url": "={{ $json.webhookUrl }}"
      }
      """
    And the connections "Webhook -> Echo"

  Scenario: An active workflow answers on its production URL
    Given the workflow is active
    When the next request has the headers:
      | user-agent | bdd-client |
    And I send a POST request to "/webhook/orders?source=shop" with body:
      """
      {"sku": "A-1"}
      """
    Then the response status is 200
    And the response JSON is:
      """
      {"sku": "A-1", "source": "shop", "agent": "bdd-client", "mode": "production", "url": "$regex:/webhook/orders$"}
      """
    And I wait for the execution to finish
    And the execution succeeds
    And the execution mode is "webhook"

  Scenario: An inactive workflow's webhook is not registered
    Given the workflow is created
    When I send a POST request to "/webhook/orders" with body:
      """
      {}
      """
    Then the response status is 404
    And the response body contains "not registered"

  Scenario: Deactivating a workflow unregisters its webhook
    Given the workflow is active
    When I deactivate the workflow
    And I send a POST request to "/webhook/orders" with body:
      """
      {}
      """
    Then the response status is 404

  Scenario: The wrong HTTP method is not accepted
    Given the workflow is active
    When I send a GET request to "/webhook/orders"
    Then the response status is 404
    And the workflow has 0 executions

  # n8n registers dynamic paths under the node's webhook id.
  Scenario: Path parameters are available to the workflow
    Given a workflow named "Users" with nodes:
      | name    | type    | parameters                                                                                 |
      | Webhook | webhook | {"httpMethod": "GET", "path": "users/:userId/orders/:orderId", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                            |
    And the node "Echo" sets the fields:
      """
      {"user": "={{ $json.params.userId }}", "order": "={{ $json.params.orderId }}"}
      """
    And the connections "Webhook -> Echo"
    And the workflow is active
    When I send a GET request to "/webhook/%{WEBHOOK_ID:Webhook}/users/42/orders/7"
    Then the response status is 200
    And the response JSON is:
      """
      {"user": "42", "order": "7"}
      """

  Scenario: Webhook paths that include the webhook id are routed too
    Given a workflow named "By id" with nodes:
      | name    | type    | parameters                                                                   |
      | Webhook | webhook | {"httpMethod": "GET", "path": "", "responseMode": "lastNode", "options": {}} |
    And the workflow is active
    When I send a GET request to "/webhook/%{WEBHOOK_ID:Webhook}"
    Then the response status is 200

  Scenario: Two active workflows cannot claim the same path and method
    Given the workflow "Orders" is active
    And a workflow named "Orders copy" with nodes:
      | name    | type    | parameters                                                                             |
      | Webhook | webhook | {"httpMethod": "POST", "path": "orders", "responseMode": "onReceived", "options": {}}   |
    When I activate the workflow "Orders copy"
    Then the response status is a client error
    And the response body contains "conflict"

  Scenario: The same path with different methods belongs to different workflows
    Given the workflow "Orders" is active
    And a workflow named "Orders lookup" with nodes:
      | name    | type    | parameters                                                                            |
      | Webhook | webhook | {"httpMethod": "GET", "path": "orders", "responseMode": "onReceived", "options": {}}   |
    And the workflow "Orders lookup" is active
    When I send a GET request to "/webhook/orders"
    Then the response status is 200
    And the workflow "Orders" has 0 executions
    And the workflow "Orders lookup" has 1 execution

  Scenario: Active webhooks survive a restart
    Given the workflow is active
    When I restart the r8r server
    And I send a POST request to "/webhook/orders" with body:
      """
      {"sku": "after-restart"}
      """
    Then the response status is 200
    And the response JSON at "sku" is "after-restart"

  Scenario: A test webhook works only while the editor is listening
    Given the workflow is created
    When I send a POST request to "/webhook-test/orders" with body:
      """
      {"sku": "T-1"}
      """
    Then the response status is 404
    When I run the workflow manually from the editor
    And I send a POST request to "/webhook-test/orders" with body:
      """
      {"sku": "T-1"}
      """
    Then the response status is 200
    And the response JSON matches:
      """
      {"sku": "T-1", "mode": "test"}
      """
    When I send a POST request to "/webhook-test/orders" with body:
      """
      {"sku": "T-2"}
      """
    Then the response status is 404

  Scenario: A binary request body is stored as binary data
    Given a workflow named "Upload" with nodes:
      | name    | type    | parameters                                                                                                      |
      | Webhook | webhook | {"httpMethod": "POST", "path": "upload", "responseMode": "lastNode", "options": {"binaryPropertyName": "file"}} |
      | Info    | set     |                                                                                                                 |
    And the node "Info" sets the fields:
      """
      {"mime": "={{ $binary.file.mimeType }}", "hasData": "={{ !!$binary.file }}"}
      """
    And the connections "Webhook -> Info"
    And the workflow is active
    When I send a POST request to "/webhook/upload" with content type "application/pdf" and body "%PDF-1.4 fake"
    Then the response status is 200
    And the response JSON is:
      """
      {"mime": "application/pdf", "hasData": true}
      """

  Scenario: Form-encoded bodies are parsed
    Given the workflow is active
    When I send a POST request to "/webhook/orders" with content type "application/x-www-form-urlencoded" and body "sku=F-9&qty=2"
    Then the response status is 200
    And the response JSON at "sku" is "F-9"

  Scenario: CORS preflight honours allowed origins
    Given a workflow named "Cors" with nodes:
      | name    | type    | parameters                                                                                                   |
      | Webhook | webhook | {"httpMethod": "POST", "path": "cors", "responseMode": "onReceived", "options": {"allowedOrigins": "https://shop.example"}} |
    And the workflow is active
    When I send an OPTIONS request to "/webhook/cors" with headers:
      | origin                        | https://shop.example |
      | access-control-request-method | POST                 |
    Then the response status is a success
    And the response header "access-control-allow-origin" is "https://shop.example"

  # n8n 2.35 answers 500 here; the spec (§8.2) requires a proper 413.
  @beyond-n8n
  Scenario: Request bodies above the payload limit are rejected
    Given the environment variable "N8N_PAYLOAD_SIZE_MAX" is "1"
    And I restart the r8r server
    And the workflow is active
    When I send a POST request to "/webhook/orders" with a JSON body of 2 MiB
    Then the response status is 413
    And the workflow has 0 executions
