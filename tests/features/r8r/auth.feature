@r8r
Feature: Authentication and registration
  The first user registers freely; afterwards registration is closed unless
  explicitly opened. Login returns a JWT; failures never reveal whether an
  email exists.

  Scenario: The first user can register
    When I send a POST request to "/rest/auth/register" with JSON:
      """
      {"email": "first@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 201
    And I remember the JSON at "/token" as "token"

  Scenario: Registration closes after the first user
    Given I am logged in as "owner@example.com"
    When I send a POST request to "/rest/auth/register" with JSON:
      """
      {"email": "second@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 403

  Scenario: A registered user can log in
    Given I am logged in as "login@example.com"
    When I send a POST request to "/rest/auth/login" with JSON:
      """
      {"email": "login@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 200
    And I remember the JSON at "/token" as "token"

  Scenario Outline: Failed logins look the same whether or not the email exists
    Given I am logged in as "real@example.com"
    When I send a POST request to "/rest/auth/login" with JSON:
      """
      {"email": "<email>", "password": "<password>"}
      """
    Then the response status is 401

    Examples:
      | email               | password              |
      | real@example.com    | wrong-password        |
      | nobody@example.com  | correct-horse-battery |

  Scenario: Protected endpoints require a session
    Given I am not logged in
    When I send a GET request to "/rest/credentials"
    Then the response status is 401

  Scenario: The health endpoint is public
    Given I am not logged in
    When I send a GET request to "/health"
    Then the response status is 200
    And the response body contains "ok"

  Scenario: Unknown API paths are 404, not the editor page
    Given I am logged in as "paths@example.com"
    When I send a GET request to "/rest/does-not-exist"
    Then the response status is 404
