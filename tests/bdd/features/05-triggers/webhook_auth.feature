@spec-6.3 @phase-2 @security
Feature: Webhook authentication
  Webhooks can require basic auth, a header or a JWT, using credentials.

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: Header auth rejects a missing or wrong header and accepts the right one
    Given the credential "Webhook header" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Hook-Token", "value": "hook-secret-123"}
      """
    And a workflow named "Header protected" with nodes:
      | name    | type    | parameters                                                                                                         |
      | Webhook | webhook | {"httpMethod": "POST", "path": "guarded", "authentication": "headerAuth", "responseMode": "onReceived", "options": {}} |
    And the node "Webhook" uses the "httpHeaderAuth" credential "Webhook header"
    And the workflow is active
    When I send a POST request to "/webhook/guarded"
    Then the response status is 403
    When I send a POST request to "/webhook/guarded" with headers:
      | X-Hook-Token | wrong |
    Then the response status is 403
    When I send a POST request to "/webhook/guarded" with headers:
      | X-Hook-Token | hook-secret-123 |
    Then the response status is 200
    And the workflow has 1 execution

  Scenario: Basic auth challenges unauthenticated callers
    Given the credential "Webhook basic" of type "httpBasicAuth" with the data:
      """
      {"user": "hook", "password": "pa55"}
      """
    And a workflow named "Basic protected" with nodes:
      | name    | type    | parameters                                                                                                        |
      | Webhook | webhook | {"httpMethod": "GET", "path": "basic", "authentication": "basicAuth", "responseMode": "onReceived", "options": {}} |
    And the node "Webhook" uses the "httpBasicAuth" credential "Webhook basic"
    And the workflow is active
    When I send a GET request to "/webhook/basic"
    Then the response status is 401
    And the response header "www-authenticate" contains "Basic"
    When I send a GET request to "/webhook/basic" with headers:
      | authorization | Basic aG9vazp3cm9uZw== |
    Then the response status is 401
    When I send a GET request to "/webhook/basic" with headers:
      | authorization | Basic aG9vazpwYTU1 |
    Then the response status is 200

  Scenario: JWT auth accepts only tokens signed with the configured secret
    Given the credential "Webhook JWT" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "jwt-secret", "algorithm": "HS256"}
      """
    And a workflow named "JWT protected" with nodes:
      | name    | type    | parameters                                                                                                    |
      | Webhook | webhook | {"httpMethod": "GET", "path": "jwt", "authentication": "jwtAuth", "responseMode": "lastNode", "options": {}}  |
    And the node "Webhook" uses the "jwtAuth" credential "Webhook JWT"
    And the workflow is active
    When I send a GET request to "/webhook/jwt" with headers:
      | authorization | Bearer not-a-jwt |
    Then the response status is 403
    When I send a GET request to "/webhook/jwt" with a bearer JWT signed with "some-other-secret"
    Then the response status is 403
    When I send a GET request to "/webhook/jwt" with a bearer JWT signed with "jwt-secret"
    Then the response status is 200

  # n8n stores the request headers, auth header included, in the webhook
  # item. The spec (§8.2) requires credential data to be redacted.
  @beyond-n8n
  Scenario: Credentials protecting a webhook never appear in its execution data
    Given the credential "Webhook header" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Hook-Token", "value": "hook-secret-456"}
      """
    And a workflow named "No leak" with nodes:
      | name    | type    | parameters                                                                                                         |
      | Webhook | webhook | {"httpMethod": "POST", "path": "noleak", "authentication": "headerAuth", "responseMode": "onReceived", "options": {}} |
    And the node "Webhook" uses the "httpHeaderAuth" credential "Webhook header"
    And the workflow is active
    When I send a POST request to "/webhook/noleak" with headers:
      | X-Hook-Token | hook-secret-456 |
    And I wait for the execution to finish
    Then the execution data does not contain "hook-secret-456"
