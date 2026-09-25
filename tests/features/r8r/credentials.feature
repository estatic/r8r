@r8r
Feature: Credential management
  Credentials are encrypted at rest, never returned to the browser, editable
  without re-entering unchanged secrets, and cannot be deleted while a
  workflow or tool uses them.

  Background:
    Given I am logged in as "cred@example.com"

  Scenario: Create a credential; secrets never come back
    When I send a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Bot", "credential_type": "telegramApi", "data": {"bot_token": "123:SECRET"}}
      """
    Then the response status is 201
    And the response body does not contain "123:SECRET"
    And I remember the JSON at "/id" as "cred"
    When I send a GET request to "/rest/credentials/{cred}"
    Then the response status is 200
    And the response body does not contain "123:SECRET"
    And the JSON at "/fields" equals:
      """
      {}
      """

  Scenario: Non-secret fields are shown for editing
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Header key", "credential_type": "apiKeyHeader", "data": {"header_name": "X-Key", "value": "top-secret"}}
      """
    And I remember the JSON at "/id" as "cred"
    When I send a GET request to "/rest/credentials/{cred}"
    Then the JSON at "/fields/header_name" is "X-Key"
    And there is no JSON at "/fields/value"
    And the response body does not contain "top-secret"

  Scenario: The credential list never includes data
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Listed", "credential_type": "bearerToken", "data": {"token": "tok-SECRET"}}
      """
    When I send a GET request to "/rest/credentials"
    Then the response status is 200
    And the JSON at "/0/name" is "Listed"
    And the JSON at "/0/used_by" is 0
    And the response body does not contain "tok-SECRET"

  Scenario: Renaming keeps the stored secret
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Old", "credential_type": "telegramApi", "data": {"bot_token": "123:ABC"}}
      """
    And I remember the JSON at "/id" as "cred"
    When I send a PATCH request to "/rest/credentials/{cred}" with JSON:
      """
      {"name": "  New  ", "data": {"bot_token": ""}}
      """
    Then the response status is 200
    And the JSON at "/name" is "New"
    And the JSON at "/credential_type" is "telegramApi"

  Scenario Outline: Invalid edits are rejected
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Edit me", "credential_type": "telegramApi", "data": {"bot_token": "x"}}
      """
    And I remember the JSON at "/id" as "cred"
    When I send a PATCH request to "/rest/credentials/{cred}" with JSON:
      """
      <body>
      """
    Then the response status is <status>

    Examples:
      | body                         | status |
      | {"name": "   "}              | 400    |
      | {"data": "not an object"}    | 400    |

  Scenario: Editing or deleting a missing credential
    When I send a PATCH request to "/rest/credentials/00000000-0000-0000-0000-000000000009" with JSON:
      """
      {"name": "x"}
      """
    Then the response status is 404
    When I send a DELETE request to "/rest/credentials/00000000-0000-0000-0000-000000000009"
    Then the response status is 404

  Scenario: A credential used by a workflow cannot be deleted
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "Used", "credential_type": "bearerToken", "data": {"token": "t"}}
      """
    And I remember the JSON at "/id" as "cred"
    And I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "uses-cred", "nodes": [{"id": "h", "node_type": "core.httpRequest", "position": [0, 0],
        "parameters": {"url": "https://example.com", "auth": {"type": "bearer", "credential_id": "{cred}"}}}], "connections": []}
      """
    And I remember the JSON at "/id" as "wf"
    When I send a DELETE request to "/rest/credentials/{cred}"
    Then the response status is 409
    And the JSON at "/error" is "credential is in use"
    And the JSON at "/workflows/0/name" is "uses-cred"
    When I send a DELETE request to "/rest/workflows/{wf}"
    And I send a DELETE request to "/rest/credentials/{cred}"
    Then the response status is 204

  Scenario: Credential forms are described by type
    When I send a GET request to "/rest/credential-types"
    Then the response status is 200
    And the response body contains "telegramApi"
    And the response body contains "bearerToken"
