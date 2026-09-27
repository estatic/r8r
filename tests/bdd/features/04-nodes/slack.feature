@spec-6.6 @phase-4 @node-slack
Feature: Slack node
  Consumes the Slack Web API (v2.2/2.3, as the n8n 2.35.7 editor creates the
  node) against the `slackApi` (access token) and `slackOAuth2Api`
  credentials. Only the most-used resources/operations are implemented
  natively; anything else fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "Bot Token" of type "slackApi" with the data:
      """
      {"accessToken": "xoxb-test-token", "url": "%{MOCK_URL}/api"}
      """

  Scenario: Posting a text message hits the exact request and appends the workflow attribution
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1699999999.000100", "message": {"type": "message", "text": "Hi there"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {
        "authentication": "accessToken",
        "resource": "message",
        "operation": "post",
        "select": "channel",
        "channelId": "C123",
        "messageType": "text",
        "text": "Hi there",
        "otherOptions": {"mrkdwn": false, "link_names": true, "unfurl_links": true}
      }
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/api/chat.postMessage"
    And the last request to "/api/chat.postMessage" had the header "authorization" equal to "Bearer xoxb-test-token"
    And the last request to "/api/chat.postMessage" had a JSON body matching:
      """
      {"channel": "C123", "mrkdwn": false, "link_names": true, "unfurl_links": true}
      """
    And the last request to "/api/chat.postMessage" had a JSON body matching:
      """
      {"text": "$contains:Hi there\n_Automated with this <"}
      """
    And the last request to "/api/chat.postMessage" had a JSON body matching:
      """
      {"text": "$contains:utm_campaign=n8n-nodes-base.slack|n8n workflow>_"}
      """
    And the node "Post" outputs:
      """
      [{"ok": true, "channel": "C123", "message": {"type": "message", "text": "Hi there"}, "message_timestamp": "1699999999.000100"}]
      """

  Scenario: Posting a threaded reply with broadcast, and no attribution when disabled
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1700000001.000200"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Reply | slack         |
    And the node "Reply" has parameters:
      """
      {
        "resource": "message",
        "operation": "post",
        "select": "channel",
        "channelId": "C123",
        "messageType": "text",
        "text": "On it",
        "otherOptions": {
          "includeLinkToWorkflow": false,
          "thread_ts": {"replyValues": {"thread_ts": "1699999999.000100", "reply_broadcast": true}}
        }
      }
      """
    And the node "Reply" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Reply"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.postMessage" had a JSON body matching:
      """
      {"channel": "C123", "text": "On it", "thread_ts": "1699999999.000100", "reply_broadcast": true}
      """

  Scenario: Posting a Blocks message appends the attribution block
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1700000002.000300"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Blocks | slack         |
    And the node "Blocks" has parameters:
      """
      {
        "resource": "message",
        "operation": "post",
        "select": "channel",
        "channelId": "C123",
        "messageType": "block",
        "blocksUi": "{\"blocks\": [{\"type\": \"section\", \"text\": {\"type\": \"mrkdwn\", \"text\": \"Deploy done\"}}]}",
        "text": "Deploy notification",
        "otherOptions": {}
      }
      """
    And the node "Blocks" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Blocks"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.postMessage" had a JSON body matching:
      """
      {
        "channel": "C123",
        "text": "Deploy notification",
        "blocks": [
          {"type": "section", "text": {"type": "mrkdwn", "text": "Deploy done"}},
          {"type": "section", "text": {"type": "mrkdwn", "text": "$contains:_Automated with this <"}}
        ]
      }
      """

  Scenario: Updating a message
    Given the mock service responds to POST "/api/chat.update" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1699999999.000100", "text": "edited"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Update | slack         |
    And the node "Update" has parameters:
      """
      {"resource": "message", "operation": "update", "channelId": "C123", "ts": "1699999999.000100", "messageType": "text", "text": "edited", "updateFields": {}}
      """
    And the node "Update" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Update"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.update" had a JSON body matching:
      """
      {"channel": "C123", "ts": "1699999999.000100"}
      """
    And the last request to "/api/chat.update" had a JSON body matching:
      """
      {"text": "$contains:edited\n_Automated with this <"}
      """
    And the node "Update" outputs:
      """
      [{"ok": true, "channel": "C123", "text": "edited", "message_timestamp": "1699999999.000100"}]
      """

  Scenario: Deleting a message
    Given the mock service responds to POST "/api/chat.delete" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1699999999.000100"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | slack         |
    And the node "Delete" has parameters:
      """
      {"resource": "message", "operation": "delete", "select": "channel", "channelId": "C123", "timestamp": "1699999999.000100"}
      """
    And the node "Delete" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.delete" had a JSON body matching:
      """
      {"channel": "C123", "ts": "1699999999.000100"}
      """
    And the node "Delete" outputs:
      """
      [{"ok": true, "channel": "C123", "message_timestamp": "1699999999.000100"}]
      """

  Scenario: Getting a message permalink
    Given the mock service responds to GET "/api/chat.getPermalink" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "permalink": "https://example.slack.com/archives/C123/p1699999999000100"}
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | Permalink | slack         |
    And the node "Permalink" has parameters:
      """
      {"resource": "message", "operation": "getPermalink", "channelId": "C123", "timestamp": "1699999999.000100"}
      """
    And the node "Permalink" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Permalink"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.getPermalink" had the query parameter "channel" equal to "C123"
    And the last request to "/api/chat.getPermalink" had the query parameter "message_ts" equal to "1699999999.000100"
    And the node "Permalink" outputs:
      """
      [{"ok": true, "channel": "C123", "permalink": "https://example.slack.com/archives/C123/p1699999999000100"}]
      """

  Scenario: Searching messages (pre-2.7 search.messages)
    Given the mock service responds to POST "/api/search.messages" with status 200 and body:
      """
      {"ok": true, "messages": {"matches": [{"text": "found it", "ts": "1.1"}], "total": 1}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Search | slack         |
    And the node "Search" has parameters:
      """
      {"resource": "message", "operation": "search", "query": "found it", "sort": "desc", "returnAll": false, "limit": 10, "options": {}}
      """
    And the node "Search" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Search"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/search.messages" had the query parameter "query" equal to "found it"
    And the last request to "/api/search.messages" had the query parameter "count" equal to "10"
    And the node "Search" outputs:
      """
      [{"text": "found it", "ts": "1.1"}]
      """

  Scenario: Creating a channel strips a leading '#' and sets visibility
    Given the mock service responds to POST "/api/conversations.create" with status 200 and body:
      """
      {"ok": true, "channel": {"id": "C999", "name": "project-x", "is_private": true}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | slack         |
    And the node "Create" has parameters:
      """
      {"resource": "channel", "operation": "create", "channelId": "#project-x", "channelVisibility": "private"}
      """
    And the node "Create" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/conversations.create" had a JSON body matching:
      """
      {"name": "project-x", "is_private": true}
      """
    And the node "Create" outputs:
      """
      [{"id": "C999", "name": "project-x", "is_private": true}]
      """

  Scenario: Getting a channel's message history
    Given the mock service responds to GET "/api/conversations.history" with status 200 and body:
      """
      {"ok": true, "messages": [{"type": "message", "text": "hi", "ts": "111.222"}], "has_more": false}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | History | slack         |
    And the node "History" has parameters:
      """
      {"resource": "channel", "operation": "history", "channelId": "C123", "returnAll": false, "limit": 20, "filters": {}}
      """
    And the node "History" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> History"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/conversations.history" had the query parameter "channel" equal to "C123"
    And the last request to "/api/conversations.history" had the query parameter "limit" equal to "20"
    And the node "History" outputs:
      """
      [{"type": "message", "text": "hi", "ts": "111.222"}]
      """

  Scenario: Inviting users to a channel
    Given the mock service responds to POST "/api/conversations.invite" with status 200 and body:
      """
      {"ok": true, "channel": {"id": "C123", "name": "general"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Invite | slack         |
    And the node "Invite" has parameters:
      """
      {"resource": "channel", "operation": "invite", "channelId": "C123", "userIds": ["U1", "U2"]}
      """
    And the node "Invite" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Invite"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/conversations.invite" had a JSON body matching:
      """
      {"channel": "C123", "users": "U1,U2"}
      """
    And the node "Invite" outputs:
      """
      [{"id": "C123", "name": "general"}]
      """

  Scenario: Getting information about a user
    Given the mock service responds to GET "/api/users.info" with status 200 and body:
      """
      {"ok": true, "user": {"id": "U1", "name": "alice", "real_name": "Alice A."}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Info  | slack         |
    And the node "Info" has parameters:
      """
      {"resource": "user", "operation": "info", "user": {"__rl": true, "mode": "id", "value": "U1"}}
      """
    And the node "Info" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Info"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/users.info" had the query parameter "user" equal to "U1"
    And the node "Info" outputs:
      """
      [{"id": "U1", "name": "alice", "real_name": "Alice A."}]
      """

  Scenario: Adding a reaction to a message
    Given the mock service responds to POST "/api/reactions.add" with status 200 and body:
      """
      {"ok": true}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | React | slack         |
    And the node "React" has parameters:
      """
      {"resource": "reaction", "operation": "add", "channelId": "C123", "timestamp": "1699999999.000100", "name": "+1"}
      """
    And the node "React" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> React"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/reactions.add" had a JSON body matching:
      """
      {"channel": "C123", "name": "+1", "timestamp": "1699999999.000100"}
      """
    And the node "React" outputs:
      """
      [{"ok": true}]
      """

  Scenario: Uploading a file via the v2 external-upload flow
    Given the mock service responds to GET "/api/files.getUploadURLExternal" with status 200 and body:
      """
      {"ok": true, "upload_url": "%{MOCK_URL}/upload/abc123", "file_id": "F123"}
      """
    And the mock service responds to POST "/upload/abc123" with status 200
    And the mock service responds to POST "/api/files.completeUploadExternal" with status 200 and body:
      """
      {"ok": true, "files": [{"id": "F123", "title": "report.txt"}]}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Upload | slack         |
    And the node "Upload" has parameters:
      """
      {"resource": "file", "operation": "upload", "binaryPropertyName": "data", "options": {"channelId": "C123", "title": "report.txt"}}
      """
    And the node "Upload" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Upload"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "hello file" and mime type "text/plain"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/files.getUploadURLExternal" had the query parameter "filename" equal to "data.txt"
    And the last request to "/api/files.completeUploadExternal" had a JSON body matching:
      """
      {"channel_id": "C123", "files": [{"id": "F123", "title": "report.txt"}]}
      """
    And the node "Upload" outputs:
      """
      [{"id": "F123", "title": "report.txt"}]
      """

  Scenario: Slack's ok:false response becomes a clear NodeApiError
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": false, "error": "channel_not_found"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C999", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "channel_not_found"

  Scenario: A missing OAuth scope maps to a clear message
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": false, "error": "missing_scope", "needed": "chat:write"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "missing required Oauth Scopes"

  Scenario: A 429 from Slack becomes a clear error
    Given the mock service responds to POST "/api/chat.postMessage" with status 429
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "status code 429"

  Scenario: A 500 from Slack becomes a clear error
    Given the mock service responds to POST "/api/chat.postMessage" with status 500
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "status code 500"

  Scenario: A missing Slack credential fails with a clear message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: The access token never appears in the execution data
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": false, "error": "not_authed"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "xoxb-test-token"

  Scenario: continueOnFail turns a Slack error into an error item instead of failing the run
    Given the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": false, "error": "channel_not_found"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"resource": "message", "operation": "post", "select": "channel", "channelId": "C999", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Post" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution succeeds
    And the node "Post" outputs:
      """
      [{"error": "$contains:channel_not_found"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Star  | slack         |
    And the node "Star" has parameters:
      """
      {"resource": "star", "operation": "add"}
      """
    And the node "Star" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Star"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Get   | slack         |
    And the node "Get" has parameters:
      """
      {"resource": "file", "operation": "get", "fileId": "F1"}
      """
    And the node "Get" uses the "slackApi" credential "Bot Token"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: Posting a message using OAuth2 authentication
    Given the credential "OAuth Bot" of type "slackOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}/api", "oauthTokenData": {"access_token": "xoxp-oauth-token"}}
      """
    And the mock service responds to POST "/api/chat.postMessage" with status 200 and body:
      """
      {"ok": true, "channel": "C123", "ts": "1700000003.000400"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"authentication": "oAuth2", "resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {"includeLinkToWorkflow": false}}
      """
    And the node "Post" uses the "slackOAuth2Api" credential "OAuth Bot"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/api/chat.postMessage" had the header "authorization" equal to "Bearer xoxp-oauth-token"

  Scenario: A Slack OAuth2 credential that was never connected fails with a clear message
    Given the credential "Unconnected" of type "slackOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}/api"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Post  | slack         |
    And the node "Post" has parameters:
      """
      {"authentication": "oAuth2", "resource": "message", "operation": "post", "select": "channel", "channelId": "C123", "messageType": "text", "text": "hi", "otherOptions": {}}
      """
    And the node "Post" uses the "slackOAuth2Api" credential "Unconnected"
    And the connections "Start -> Post"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not connected"
