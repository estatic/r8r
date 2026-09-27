@r8r-only @legacy-api
Feature: Authentication and registration
  The first user registers freely; afterwards registration is closed unless
  explicitly opened. Login returns a JWT; failures never reveal whether an
  email exists.

  Background:
    Given a running r8r server

  Scenario: The first user can register
    When I send a POST request to "/rest/r8r/auth/register" with body:
      """
      {"email": "first@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 201
    And I remember the response JSON at "token" as "token"

  Scenario: Registration closes after the first user
    Given I am logged in to the legacy r8r API as "owner@example.com"
    When I send a POST request to "/rest/r8r/auth/register" with body:
      """
      {"email": "second@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 403

  Scenario: A registered user can log in
    Given I am logged in to the legacy r8r API as "login@example.com"
    When I send a POST request to "/rest/r8r/auth/login" with body:
      """
      {"email": "login@example.com", "password": "correct-horse-battery"}
      """
    Then the response status is 200
    And I remember the response JSON at "token" as "token"

  Scenario Outline: Failed logins look the same whether or not the email exists
    Given I am logged in to the legacy r8r API as "real@example.com"
    When I send a POST request to "/rest/r8r/auth/login" with body:
      """
      {"email": "<email>", "password": "<password>"}
      """
    Then the response status is 401

    Examples:
      | email               | password              |
      | real@example.com    | wrong-password        |
      | nobody@example.com  | correct-horse-battery |

  Scenario: Protected endpoints require a session
    Given I am not authenticated
    When I send a GET request to "/rest/r8r/credentials"
    Then the response status is 401

  Scenario: The health endpoint is public
    Given I am not authenticated
    When I send a GET request to "/health"
    Then the response status is 200
    And the response body contains "ok"

  Scenario: Unknown API paths are 404, not the editor page
    Given I am logged in to the legacy r8r API as "paths@example.com"
    When I send a GET request to "/rest/r8r/does-not-exist"
    Then the response status is 404
