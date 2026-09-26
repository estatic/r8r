@spec-6.9 @spec-6.10 @spec-6.11 @phase-2
Feature: Editor REST API and authentication
  `/rest/*` is the API the pinned n8n editor build calls. Responses are
  wrapped in `{ "data": ... }`. Sessions are an `n8n-auth` JWT cookie
  (HttpOnly, SameSite=Lax).

  Scenario: The first visit sets up the owner account
    Given a running r8r server
    When I send a GET request to "/rest/settings"
    Then the response status is 200
    And the response JSON matches:
      """
      {"data": {"userManagement": {"showSetupOnFirstLoad": true}}}
      """
    When I send a POST request to "/rest/owner/setup" with body:
      """
      {"email": "owner@example.com", "firstName": "Olivia", "lastName": "Owner", "password": "Passw0rd!Passw0rd"}
      """
    Then the response status is 200
    And the response sets the session cookie "n8n-auth" with the attributes "HttpOnly, SameSite=Lax"
    And the response JSON matches:
      """
      {"data": {"email": "owner@example.com", "role": "global:owner"}}
      """

  Scenario: The owner can be set up only once
    Given a running r8r server with an owner account
    And I am not authenticated
    When I send a POST request to "/rest/owner/setup" with body:
      """
      {"email": "intruder@example.com", "firstName": "I", "lastName": "N", "password": "Passw0rd!Passw0rd"}
      """
    Then the response status is 400

  Scenario: Weak passwords are refused at setup
    Given a running r8r server
    When I send a POST request to "/rest/owner/setup" with body:
      """
      {"email": "owner@example.com", "firstName": "O", "lastName": "O", "password": "short"}
      """
    Then the response status is 400

  Scenario: Logging in returns the user and a session cookie
    Given a running r8r server with an owner account
    When I log in as the owner
    Then the response status is 200
    And the response sets the session cookie "n8n-auth" with the attributes "HttpOnly, SameSite=Lax"
    And the response JSON matches:
      """
      {"data": {"email": "owner@example.com", "firstName": "Olivia"}}
      """

  Scenario: A wrong password is rejected
    Given a running r8r server with an owner account
    When I log in as the owner with the password "wrong-password"
    Then the response status is 401

  Scenario: Session cookies are marked Secure behind TLS
    Given the environment variable "N8N_PROTOCOL" is "https"
    And the environment variable "N8N_SECURE_COOKIE" is "true"
    And a running r8r server with an owner account
    When I log in as the owner
    Then the response sets the session cookie "n8n-auth" with the attributes "HttpOnly, SameSite=Lax, Secure"

  Scenario: The editor API requires a session
    Given a running r8r server with an owner account
    And I am not authenticated
    When I send a GET request to "/rest/workflows"
    Then the response status is 401

  Scenario: The current user is returned for a valid session
    Given a running r8r server with an owner account
    When I send a GET request to "/rest/login"
    Then the response status is 200
    And the response JSON at "data.email" is "owner@example.com"

  Scenario: Logging out ends the session
    Given a running r8r server with an owner account
    When I send a POST request to "/rest/logout"
    Then the response status is 200
    And the response header "set-cookie" contains "n8n-auth=;"

  Scenario: Workflows are saved and listed through the editor API
    Given a running r8r server with an owner account
    When I send a POST request to "/rest/workflows" with body:
      """
      {"name": "From the editor", "nodes": [{"id": "1", "name": "Start", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}}], "connections": {}, "settings": {"executionOrder": "v1"}, "active": false}
      """
    Then the response status is 200
    And the response JSON matches:
      """
      {"data": {"id": "$string", "name": "From the editor", "versionId": "$string"}}
      """
    When I send a GET request to "/rest/workflows"
    Then the response JSON at "data" contains an element matching:
      """
      {"name": "From the editor"}
      """

  Scenario: Editor settings expose the webhook endpoints
    Given a running r8r server with an owner account
    When I send a GET request to "/rest/settings"
    Then the response JSON matches:
      """
      {"data": {"endpointWebhook": "webhook", "endpointWebhookTest": "webhook-test", "endpointWebhookWaiting": "webhook-waiting", "executionMode": "regular", "timezone": "$string", "versionCli": "$string"}}
      """

  Scenario: Node type descriptions are served for the editor
    Given a running r8r server with an owner account
    When I send a GET request to "/types/nodes.json"
    Then the response status is 200
    And the response JSON at "" contains an element matching:
      """
      {"name": "n8n-nodes-base.set", "displayName": "Edit Fields (Set)", "version": "$any", "properties": "$nonempty", "inputs": "$any", "outputs": "$any", "defaults": {"name": "$string"}}
      """
    And the response JSON at "" contains an element matching:
      """
      {"name": "n8n-nodes-base.httpRequest", "group": "$any", "credentials": "$any"}
      """

  Scenario: Every node on the native GA list is described
    Given a running r8r server with an owner account
    When I send a GET request to "/types/nodes.json"
    Then the node types list includes:
      """
      n8n-nodes-base.manualTrigger
      n8n-nodes-base.scheduleTrigger
      n8n-nodes-base.webhook
      n8n-nodes-base.formTrigger
      n8n-nodes-base.errorTrigger
      n8n-nodes-base.executeWorkflowTrigger
      n8n-nodes-base.httpRequest
      n8n-nodes-base.code
      n8n-nodes-base.set
      n8n-nodes-base.if
      n8n-nodes-base.switch
      n8n-nodes-base.filter
      n8n-nodes-base.merge
      n8n-nodes-base.splitInBatches
      n8n-nodes-base.splitOut
      n8n-nodes-base.aggregate
      n8n-nodes-base.summarize
      n8n-nodes-base.sort
      n8n-nodes-base.limit
      n8n-nodes-base.removeDuplicates
      n8n-nodes-base.compareDatasets
      n8n-nodes-base.wait
      n8n-nodes-base.respondToWebhook
      n8n-nodes-base.executeWorkflow
      n8n-nodes-base.noOp
      n8n-nodes-base.stopAndError
      n8n-nodes-base.dateTime
      n8n-nodes-base.crypto
      n8n-nodes-base.xml
      n8n-nodes-base.html
      n8n-nodes-base.markdown
      n8n-nodes-base.jwt
      n8n-nodes-base.compression
      n8n-nodes-base.extractFromFile
      n8n-nodes-base.convertToFile
      n8n-nodes-base.readWriteFile
      n8n-nodes-base.emailSend
      n8n-nodes-base.emailReadImap
      n8n-nodes-base.ftp
      n8n-nodes-base.ssh
      n8n-nodes-base.postgres
      n8n-nodes-base.mySql
      n8n-nodes-base.microsoftSql
      n8n-nodes-base.mongoDb
      n8n-nodes-base.redis
      n8n-nodes-base.rabbitmq
      n8n-nodes-base.kafka
      n8n-nodes-base.mqtt
      n8n-nodes-base.slack
      n8n-nodes-base.googleSheets
      n8n-nodes-base.gmail
      n8n-nodes-base.googleDrive
      n8n-nodes-base.notion
      n8n-nodes-base.airtable
      n8n-nodes-base.github
      n8n-nodes-base.telegram
      n8n-nodes-base.discord
      @n8n/n8n-nodes-langchain.openAi
      @n8n/n8n-nodes-langchain.agent
      @n8n/n8n-nodes-langchain.lmChatOpenAi
      """

  Scenario: Unknown editor endpoints answer 404
    Given a running r8r server with an owner account
    When I send a GET request to "/rest/this-endpoint-does-not-exist"
    Then the response status is 404
