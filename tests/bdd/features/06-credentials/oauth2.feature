@spec-6.5 @phase-2
Feature: OAuth2 credentials
  OAuth2 credentials fetch tokens (client credentials here; authorization
  code and PKCE go through `/rest/oauth2-credential/callback`), attach them
  as bearer tokens, and refresh automatically when an API answers 401.

  Background:
    Given a mock HTTP service
    And the credential "Partner OAuth" of type "oAuth2Api" with the data:
      """
      {
        "grantType": "clientCredentials",
        "accessTokenUrl": "%{MOCK_URL}/oauth/token",
        "clientId": "client-1",
        "clientSecret": "client-secret-1",
        "scope": "read",
        "authentication": "body"
      }
      """
    And a workflow with nodes:
      | name  | type          | parameters                                                                                                    |
      | Start | manualTrigger |                                                                                                               |
      | Call  | httpRequest   | {"url": "%{MOCK_URL}/api/data", "authentication": "genericCredentialType", "genericAuthType": "oAuth2Api", "options": {}} |
    And the node "Call" uses the "oAuth2Api" credential "Partner OAuth"
    And the connections "Start -> Call"

  Scenario: Client credentials grant fetches a token and uses it
    Given the mock service issues the OAuth2 access tokens "token-1" in order at "/oauth/token"
    And the mock service only accepts the bearer token "token-1" on GET "/api/data"
    When I execute the workflow
    Then the execution succeeds
    And the node "Call" outputs:
      """
      [{"ok": true}]
      """
    And the mock service received 1 request to "/oauth/token"

  Scenario: A 401 triggers one token refresh and a retry
    Given the mock service issues the OAuth2 access tokens "expired-token, fresh-token" in order at "/oauth/token"
    And the mock service only accepts the bearer token "fresh-token" on GET "/api/data"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/oauth/token"
    And the mock service received 2 requests to "/api/data"
    And the 2nd request to "/api/data" had the header "authorization" equal to "Bearer fresh-token"

  Scenario: A second 401 after refreshing fails the node
    Given the mock service issues the OAuth2 access tokens "bad-1, bad-2" in order at "/oauth/token"
    And the mock service only accepts the bearer token "never-issued" on GET "/api/data"
    When I execute the workflow
    Then the execution fails
    And the mock service received 2 requests to "/api/data"

  @phase-2
  Scenario: The OAuth2 callback endpoint exists for authorization-code flows
    Given a running r8r server with an owner account
    When I send a GET request to "/rest/oauth2-credential/callback?code=abc&state=not-a-valid-state"
    Then the response body contains "state"
