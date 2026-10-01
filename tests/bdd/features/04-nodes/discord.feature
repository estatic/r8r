@spec-6.6 @phase-4 @node-discord
Feature: Discord node
  Consumes the Discord API v10 (`nodes/Discord/v2`, as the n8n 2.35.7 editor
  creates the node) against the `discordBotApi`, `discordOAuth2Api`, and
  `discordWebhookApi` credentials. `sendAndWait` is out of scope; anything
  unimplemented fails with a clear message.

  Background:
    Given a mock HTTP service

  # ---- channel -------------------------------------------------------------

  Scenario: Creating a channel
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/guilds/G1/channels" with status 200 and body:
      """
      {"id": "C1", "name": "general", "type": 0}
      """
    And a workflow with nodes:
      | name   | type    |
      | Start  | manualTrigger |
      | Create | discord |
    And the node "Create" has parameters:
      """
      {"resource": "channel", "operation": "create", "guildId": "G1", "name": "general", "type": "0", "options": {}}
      """
    And the node "Create" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/guilds/G1/channels" had a JSON body matching:
      """
      {"name": "general", "type": "0"}
      """
    And the node "Create" outputs:
      """
      [{"id": "C1", "name": "general", "type": 0}]
      """

  Scenario: Creating a channel under a parent category
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/guilds/G1/channels" with status 200 and body:
      """
      {"id": "C2", "name": "sub", "type": 0}
      """
    And a workflow with nodes:
      | name   | type    |
      | Start  | manualTrigger |
      | Create | discord |
    And the node "Create" has parameters:
      """
      {"resource": "channel", "operation": "create", "guildId": "G1", "name": "sub", "type": "0", "options": {"categoryId": "CAT1"}}
      """
    And the node "Create" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/guilds/G1/channels" had a JSON body matching:
      """
      {"name": "sub", "type": "0", "parent_id": "CAT1"}
      """

  Scenario: Getting a channel
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1" with status 200 and body:
      """
      {"id": "C1", "name": "general", "guild_id": "G1"}
      """
    And a workflow with nodes:
      | name  | type    |
      | Start | manualTrigger |
      | Get   | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": "C1", "name": "general", "guild_id": "G1"}]
      """

  Scenario: Getting many channels respects the limit
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/guilds/G1/channels" with status 200 and body:
      """
      [{"id": "C1", "type": 0}, {"id": "C2", "type": 2}, {"id": "C3", "type": 4}]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "channel", "operation": "getAll", "guildId": "G1", "returnAll": false, "limit": 2, "options": {}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the node "GetMany" outputs:
      """
      [{"id": "C1", "type": 0}, {"id": "C2", "type": 2}]
      """

  Scenario: Getting many channels filters by type
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/guilds/G1/channels" with status 200 and body:
      """
      [{"id": "C1", "type": 0}, {"id": "C2", "type": 2}, {"id": "C3", "type": 4}]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "channel", "operation": "getAll", "guildId": "G1", "returnAll": true, "options": {"filter": [2]}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the node "GetMany" outputs:
      """
      [{"id": "C2", "type": 2}]
      """

  Scenario: Deleting a channel
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to DELETE "/channels/C1" with status 200
    And a workflow with nodes:
      | name   | type    |
      | Start  | manualTrigger |
      | Delete | discord |
    And the node "Delete" has parameters:
      """
      {"resource": "channel", "operation": "deleteChannel", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Delete" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs:
      """
      [{"success": true}]
      """

  Scenario: Updating a channel
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to PATCH "/channels/C1" with status 200 and body:
      """
      {"id": "C1", "name": "new-name"}
      """
    And a workflow with nodes:
      | name   | type    |
      | Start  | manualTrigger |
      | Update | discord |
    And the node "Update" has parameters:
      """
      {"resource": "channel", "operation": "update", "guildId": "G1", "channelId": "C1", "name": "new-name", "options": {}}
      """
    And the node "Update" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Update"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/channels/C1" had a JSON body matching:
      """
      {"name": "new-name"}
      """
    And the node "Update" outputs:
      """
      [{"id": "C1", "name": "new-name"}]
      """

  # ---- member ---------------------------------------------------------------

  Scenario: Getting many members respects the limit
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/guilds/G1/members" with status 200 and body:
      """
      [{"user": {"id": "M1"}, "roles": []}]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "member", "operation": "getAll", "guildId": "G1", "returnAll": false, "limit": 25, "after": "", "options": {}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/guilds/G1/members" had the query parameter "limit" equal to "25"
    And the node "GetMany" outputs:
      """
      [{"user": {"id": "M1"}, "roles": []}]
      """

  Scenario: Getting many members paginates with "after" until a page comes back empty
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/guilds/G1/members" in order with:
      """
      [
        {"status": 200, "body": [{"user": {"id": "M1"}, "roles": []}, {"user": {"id": "M2"}, "roles": []}]},
        {"status": 200, "body": [{"user": {"id": "M3"}, "roles": []}]},
        {"status": 200, "body": []}
      ]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "member", "operation": "getAll", "guildId": "G1", "returnAll": true, "after": "", "options": {}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 3 requests to "/guilds/G1/members"
    And the node "GetMany" outputs:
      """
      [
        {"user": {"id": "M1"}, "roles": []},
        {"user": {"id": "M2"}, "roles": []},
        {"user": {"id": "M3"}, "roles": []}
      ]
      """

  Scenario: Adding roles to a member issues one PUT per role
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to PUT "/guilds/G1/members/U1/roles/R1" with status 200
    And the mock service responds to PUT "/guilds/G1/members/U1/roles/R2" with status 200
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | AddRole | discord |
    And the node "AddRole" has parameters:
      """
      {"resource": "member", "operation": "roleAdd", "guildId": "G1", "userId": "U1", "role": ["R1", "R2"]}
      """
    And the node "AddRole" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> AddRole"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/guilds/G1/members/U1/roles/R1"
    And the mock service received 1 requests to "/guilds/G1/members/U1/roles/R2"
    And the node "AddRole" outputs:
      """
      [{"success": true}]
      """

  Scenario: Removing a role from a member
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to DELETE "/guilds/G1/members/U1/roles/R1" with status 200
    And a workflow with nodes:
      | name       | type    |
      | Start      | manualTrigger |
      | RemoveRole | discord |
    And the node "RemoveRole" has parameters:
      """
      {"resource": "member", "operation": "roleRemove", "guildId": "G1", "userId": "U1", "role": ["R1"]}
      """
    And the node "RemoveRole" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> RemoveRole"
    When I execute the workflow
    Then the execution succeeds
    And the node "RemoveRole" outputs:
      """
      [{"success": true}]
      """

  # ---- message: send ---------------------------------------------------------

  Scenario: Sending a plain text message to a channel
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 200 and body:
      """
      {"id": "MSG1"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1", "content": "hello", "options": {}}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/channels/C1/messages" had a JSON body matching:
      """
      {"content": "hello"}
      """
    And the node "Send" outputs:
      """
      [{"id": "MSG1"}]
      """

  Scenario: Sending a message with embeds entered as fields
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 200 and body:
      """
      {"id": "MSG2"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {
        "resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1",
        "content": "", "options": {},
        "embeds": {"values": [{"inputMethod": "fields", "description": "desc", "author": "Ada", "color": "#00ff00", "title": "T", "image": "https://example.com/img.png"}]}
      }
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/channels/C1/messages" had a JSON body matching:
      """
      {
        "embeds": [
          {"description": "desc", "author": {"name": "Ada"}, "color": 65280, "title": "T", "image": {"url": "https://example.com/img.png"}}
        ]
      }
      """

  Scenario: Sending a message with a raw JSON embed
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 200 and body:
      """
      {"id": "MSG3"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {
        "resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1",
        "content": "", "options": {},
        "embeds": {"values": [{"inputMethod": "json", "json": "{\"title\": \"Hi\", \"color\": \"#ff0000\"}"}]}
      }
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/channels/C1/messages" had a JSON body matching:
      """
      {"embeds": [{"title": "Hi", "color": 16711680}]}
      """

  Scenario: Sending a message with a file attachment uploads it as multipart/form-data
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 200 and body:
      """
      {"id": "MSG4"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {
        "resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1",
        "content": "see attached", "options": {},
        "files": {"values": [{"inputFieldName": "data"}]}
      }
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "hello file" and mime type "text/plain"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/channels/C1/messages" had a multipart file field "files[0]" with filename "data.txt"
    And the node "Send" outputs:
      """
      [{"id": "MSG4"}]
      """

  Scenario: Sending a direct message to a user creates a DM channel first
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/users/@me/channels" with status 200 and body:
      """
      {"id": "DM1"}
      """
    And the mock service responds to POST "/channels/DM1/messages" with status 200 and body:
      """
      {"id": "MSG5"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "user", "userId": "U1", "content": "hi there", "options": {}}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/users/@me/channels" had a JSON body matching:
      """
      {"recipient_id": "U1"}
      """
    And the last request to "/channels/DM1/messages" had a JSON body matching:
      """
      {"content": "hi there"}
      """
    And the node "Send" outputs:
      """
      [{"id": "MSG5"}]
      """

  # ---- message: get / getAll / deleteMessage / react -------------------------

  Scenario: Getting a message with simplify drops extra fields
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1/messages/M1" with status 200 and body:
      """
      {"id": "M1", "channel_id": "C1", "author": {"id": "A1"}, "content": "hey", "timestamp": "2024-01-01T00:00:00Z", "type": 0, "pinned": false}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "message", "operation": "get", "guildId": "G1", "channelId": "C1", "messageId": "M1", "options": {"simplify": true}}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": "M1", "channel_id": "C1", "author": {"id": "A1"}, "content": "hey", "timestamp": "2024-01-01T00:00:00Z", "type": 0}]
      """

  Scenario: Getting many messages respects the limit
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1/messages" with status 200 and body:
      """
      [{"id": "M1"}, {"id": "M2"}]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "message", "operation": "getAll", "guildId": "G1", "channelId": "C1", "returnAll": false, "limit": 50, "options": {}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the node "GetMany" outputs:
      """
      [{"id": "M1"}, {"id": "M2"}]
      """

  Scenario: Getting many messages paginates with "before" until a page comes back empty
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1/messages" in order with:
      """
      [
        {"status": 200, "body": [{"id": "M3"}, {"id": "M2"}]},
        {"status": 200, "body": [{"id": "M1"}]},
        {"status": 200, "body": []}
      ]
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | GetMany | discord |
    And the node "GetMany" has parameters:
      """
      {"resource": "message", "operation": "getAll", "guildId": "G1", "channelId": "C1", "returnAll": true, "options": {}}
      """
    And the node "GetMany" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 3 requests to "/channels/C1/messages"
    And the node "GetMany" outputs:
      """
      [{"id": "M3"}, {"id": "M2"}, {"id": "M1"}]
      """

  Scenario: Deleting a message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to DELETE "/channels/C1/messages/M1" with status 200
    And a workflow with nodes:
      | name   | type    |
      | Start  | manualTrigger |
      | Delete | discord |
    And the node "Delete" has parameters:
      """
      {"resource": "message", "operation": "deleteMessage", "guildId": "G1", "channelId": "C1", "messageId": "M1"}
      """
    And the node "Delete" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs:
      """
      [{"success": true}]
      """

  Scenario: Reacting to a message with an emoji
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to PUT "/channels/C1/messages/M1/reactions/thumbsup/@me" with status 200
    And a workflow with nodes:
      | name  | type    |
      | Start | manualTrigger |
      | React | discord |
    And the node "React" has parameters:
      """
      {"resource": "message", "operation": "react", "guildId": "G1", "channelId": "C1", "messageId": "M1", "emoji": "thumbsup"}
      """
    And the node "React" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> React"
    When I execute the workflow
    Then the execution succeeds
    And the node "React" outputs:
      """
      [{"success": true}]
      """

  # ---- webhook ----------------------------------------------------------------

  Scenario: Sending a message via a webhook
    Given the credential "Hook" of type "discordWebhookApi" with the data:
      """
      {"webhookUri": "%{MOCK_URL}/webhooks/123/token"}
      """
    And the mock service responds to POST "/webhooks/123/token" with status 200 and body:
      """
      {"id": "WH1"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"authentication": "webhook", "operation": "sendLegacy", "content": "hi", "options": {}}
      """
    And the node "Send" uses the "discordWebhookApi" credential "Hook"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/webhooks/123/token" had a JSON body matching:
      """
      {"content": "hi"}
      """
    And the node "Send" outputs:
      """
      [{"id": "WH1"}]
      """

  Scenario: Sending a message via a webhook with wait adds the wait query parameter
    Given the credential "Hook" of type "discordWebhookApi" with the data:
      """
      {"webhookUri": "%{MOCK_URL}/webhooks/123/token"}
      """
    And the mock service responds to POST "/webhooks/123/token" with status 200 and body:
      """
      {"id": "WH2"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"authentication": "webhook", "operation": "sendLegacy", "content": "hi", "options": {"wait": true}}
      """
    And the node "Send" uses the "discordWebhookApi" credential "Hook"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/webhooks/123/token" had the query parameter "wait" equal to "true"

  Scenario: The webhook URL never appears in the execution data
    Given the credential "Hook" of type "discordWebhookApi" with the data:
      """
      {"webhookUri": "%{MOCK_URL}/webhooks/123/supersecrettoken"}
      """
    And the mock service responds to POST "/webhooks/123/supersecrettoken" with status 401 and body:
      """
      {"message": "401: Unauthorized"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"authentication": "webhook", "operation": "sendLegacy", "content": "hi", "options": {}}
      """
    And the node "Send" uses the "discordWebhookApi" credential "Hook"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "supersecrettoken"

  # ---- oAuth2: guild / channel access checks ----------------------------------

  Scenario: oAuth2 checks guild access via the real OAuth token, then calls the API with the credential's bot token
    Given the credential "OAuth" of type "discordOAuth2Api" with the data:
      """
      {"botToken": "oauth_bot_token", "oauthTokenData": {"access_token": "oauth_access_token"}, "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/users/@me/guilds" with status 200 and body:
      """
      [{"id": "G1", "name": "Guild One"}]
      """
    And the mock service responds to GET "/channels/C1" with status 200 and body:
      """
      {"id": "C1", "guild_id": "G1", "name": "general"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordOAuth2Api" credential "OAuth"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/users/@me/guilds" had the header "authorization" equal to "Bearer oauth_access_token"
    And the mock service received 2 requests to "/channels/C1"
    And the last request to "/channels/C1" had the header "authorization" equal to "Bot oauth_bot_token"
    And the node "Get" outputs:
      """
      [{"id": "C1", "guild_id": "G1", "name": "general"}]
      """

  Scenario: oAuth2 fails fast when the user does not have access to the selected guild
    Given the credential "OAuth" of type "discordOAuth2Api" with the data:
      """
      {"botToken": "oauth_bot_token", "oauthTokenData": {"access_token": "oauth_access_token"}, "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/users/@me/guilds" with status 200 and body:
      """
      []
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordOAuth2Api" credential "OAuth"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "You do not have access to the guild with the id G1"

  Scenario: oAuth2 denies access to a channel that belongs to a different guild
    Given the credential "OAuth" of type "discordOAuth2Api" with the data:
      """
      {"botToken": "oauth_bot_token", "oauthTokenData": {"access_token": "oauth_access_token"}, "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/users/@me/guilds" with status 200 and body:
      """
      [{"id": "G1"}]
      """
    And the mock service responds to GET "/channels/C1" with status 200 and body:
      """
      {"id": "C1", "guild_id": "G2"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordOAuth2Api" credential "OAuth"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "You do not have access to the guild with the id G2"

  Scenario: oAuth2 reports a clear error when a channel's server cannot be found
    Given the credential "OAuth" of type "discordOAuth2Api" with the data:
      """
      {"botToken": "oauth_bot_token", "oauthTokenData": {"access_token": "oauth_access_token"}, "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/users/@me/guilds" with status 200 and body:
      """
      [{"id": "G1"}]
      """
    And the mock service responds to GET "/channels/C1" with status 404 and body:
      """
      {"message": "Unknown Channel"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordOAuth2Api" credential "OAuth"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Could not find server for channel with the id C1"

  # ---- errors, auth, continueOnFail, and unsupported operations --------------

  Scenario: A 401 from Discord maps to n8n's authorization-failed message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1" with status 401 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 403 from Discord maps to n8n's forbidden message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1" with status 403 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Forbidden - perhaps check your credentials?"

  Scenario: A 404 from Discord maps to n8n's not-found message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1" with status 404 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The resource you are requesting could not be found"

  Scenario: A 429 from Discord maps to n8n's rate-limit message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 429 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1", "content": "hi", "options": {}}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The service is receiving too many requests from you"

  Scenario: A 400 with malformed embed fields names the bad parameters
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 400 and body:
      """
      {"message": "Invalid Form Body", "errors": {"embeds": {"0": {"color": {"_errors": [{"message": "bad"}]}, "title": {"_errors": [{"message": "bad"}]}}}}}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1", "content": "hi", "options": {}}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The parameters Color, Title are not properly formatted"

  Scenario: A 400 with an unknown message_reference names the reply parameter
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 400 and body:
      """
      {"message": "Invalid Form Body", "errors": {"message_reference": {"_errors": [{"message": "Unknown message"}]}}}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1", "content": "hi", "options": {"message_reference": "999"}}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The message to reply to ID can't be found"

  Scenario: A missing Discord credential fails with a clear message
    Given a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: A Discord bot credential with a blank token fails with a clear message
    Given the credential "Blank" of type "discordBotApi" with the data:
      """
      {"botToken": "", "url": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Blank"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Discord credentials are not set"

  Scenario: The bot token never appears in the execution data
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "very_secret_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to GET "/channels/C1" with status 401 and body:
      """
      {}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Get  | discord |
    And the node "Get" has parameters:
      """
      {"resource": "channel", "operation": "get", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Get" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "very_secret_bot_token"

  Scenario: continueOnFail turns a Discord error into an error item
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And the mock service responds to POST "/channels/C1/messages" with status 400 and body:
      """
      {"message": "Cannot send an empty message"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Send | discord |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "send", "guildId": "G1", "sendTo": "channel", "channelId": "C1", "content": "", "options": {}}
      """
    And the node "Send" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Send" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs:
      """
      [{"error": "Cannot send an empty message"}]
      """

  Scenario: An unimplemented operation returns a clear "not supported" message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name    | type    |
      | Start   | manualTrigger |
      | Archive | discord |
    And the node "Archive" has parameters:
      """
      {"resource": "channel", "operation": "archive", "guildId": "G1", "channelId": "C1"}
      """
    And the node "Archive" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Archive"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given the credential "Bot" of type "discordBotApi" with the data:
      """
      {"botToken": "test_bot_token", "url": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name | type    |
      | Start| manualTrigger |
      | Info | discord |
    And the node "Info" has parameters:
      """
      {"resource": "role", "operation": "list", "guildId": "G1"}
      """
    And the node "Info" uses the "discordBotApi" credential "Bot"
    And the connections "Start -> Info"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
