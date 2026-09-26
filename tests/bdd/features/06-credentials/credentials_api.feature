@spec-6.5 @spec-8.2 @phase-2
Feature: Managing credentials without exposing secrets
  Credentials are created through the API, described by n8n's credential
  type schemas, and their secret data is never returned, logged or stored
  in execution data.

  Background:
    Given a running r8r server with an owner and an API key

  Scenario: Creating a credential returns its metadata but never its data
    When I send a POST request to "/api/v1/credentials" with body:
      """
      {"name": "Billing API", "type": "httpHeaderAuth", "data": {"name": "X-Key", "value": "billing-secret-1"}}
      """
    Then the response status is 200
    And the response JSON matches:
      """
      {"id": "$string", "name": "Billing API", "type": "httpHeaderAuth", "createdAt": "$datetime"}
      """
    And the response JSON has no key "data"
    And the response body does not contain "billing-secret-1"

  Scenario: The editor API redacts secret fields
    Given the credential "Billing API" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Key", "value": "billing-secret-2"}
      """
    And I am logged in as the owner
    When I send a GET request to "/rest/credentials/%{CREDENTIAL_ID:Billing API}?includeData=true"
    Then the response status is 200
    And the response body does not contain "billing-secret-2"
    And the response JSON at "data.data.name" is "X-Key"

  Scenario: Credential data is validated against the type's schema
    When I send a POST request to "/api/v1/credentials" with body:
      """
      {"name": "Broken", "type": "httpHeaderAuth", "data": {"unexpected": true}}
      """
    Then the response status is 400

  Scenario: An unknown credential type is rejected
    When I send a POST request to "/api/v1/credentials" with body:
      """
      {"name": "Mystery", "type": "doesNotExistApi", "data": {}}
      """
    Then the response status is a client error

  Scenario: The JSON schema of a credential type is published
    When I send a GET request to "/api/v1/credentials/schema/httpHeaderAuth"
    Then the response status is 200
    And the response JSON matches:
      """
      {"type": "object", "properties": {"name": {"type": "string"}, "value": {"type": "string"}}}
      """

  Scenario: Credential type descriptions are served to the editor
    Given I am logged in as the owner
    When I send a GET request to "/types/credentials.json"
    Then the response status is 200
    And the response JSON at "" contains an element matching:
      """
      {"name": "httpHeaderAuth", "displayName": "$string", "properties": "$nonempty"}
      """

  Scenario: Deleting a credential removes it
    Given the credential "Temp" of type "httpBasicAuth" with the data:
      """
      {"user": "u", "password": "p"}
      """
    When I send a DELETE request to "/api/v1/credentials/%{CREDENTIAL_ID:Temp}"
    Then the response status is 200
    When I send a DELETE request to "/api/v1/credentials/%{CREDENTIAL_ID:Temp}"
    Then the response status is 404

  Scenario: Secrets never reach the server log
    Given the credential "Noisy" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Key", "value": "log-secret-9"}
      """
    And the environment variable "N8N_LOG_LEVEL" is "debug"
    And a mock HTTP service
    And the mock service responds to GET "/fails" with status 500
    And a workflow named "Logs secrets?" with nodes:
      | name    | type        | parameters                                                                                                                        |
      | Webhook | webhook     | {"httpMethod": "GET", "path": "noisy", "responseMode": "lastNode", "options": {}}                                                 |
      | Call    | httpRequest | {"url": "%{MOCK_URL}/fails", "authentication": "genericCredentialType", "genericAuthType": "httpHeaderAuth", "options": {}}        |
    And the node "Call" uses the "httpHeaderAuth" credential "Noisy"
    And the connections "Webhook -> Call"
    And the workflow is active
    When I send a GET request to "/webhook/noisy"
    And I wait for the execution to finish
    Then the execution fails
    And the execution data does not contain "log-secret-9"
    And the server log does not contain "log-secret-9"
