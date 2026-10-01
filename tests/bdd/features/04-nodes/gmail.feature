@spec-6.6 @phase-4 @node-gmail
Feature: Gmail node
  Consumes the Gmail v1 API (node v2/2.1/2.2, as the n8n 2.35.7 editor
  creates it) against the `gmailOAuth2` and `googleApi` (service-account
  JWT) credentials. Implements the `message` resource's send, reply, get,
  getAll, delete, markAsRead, markAsUnread, addLabels and removeLabels
  operations; the `draft` resource's create, get, getAll and delete; the
  `label` resource's create, get, getAll and delete; and the `thread`
  resource's get, getAll, delete, reply, trash, untrash, addLabels and
  removeLabels. Anything else fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "Service Account" of type "googleApi" with the data:
      """
      {
        "email": "svc@example.iam.gserviceaccount.com",
        "privateKey": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCgHQNrUetQxEOs\nNfsDt8iMM4/YRAkvAXJQNN7J03CZrB5qg/4NE9yhLA/Iy+VMX+5pBrxoXn5ScTMD\ns0g1qtRR5D2tOLsTjKC+m7sOvVzRh4vJWAXAmaWxfrCg59M1AkYvbIDHs3D5hnTq\nG2ANmA5C9l5zYCeQRH3Niama6ZcI+aaOlwGWklVfTCOBlbIjhqGTr1XYo+vEu+d7\nCZRUB1AR8AphEUQKKxVzQCftnKQAt6lIYWj0Ivt57JtYeKtmvyM1a/nIm/7VH0UH\nctZQljtVKShYF/Mx0bgugJRzaXwFdoKHHddP+eZOUlxmrYdNM4LmU+KofnbMwUpN\nXnRq7OWlAgMBAAECggEAI7NMaZ/GqQfV8f5IRaQzKIWvr2A+Lvey2izrtxM44l7l\nGklrelWBJoO5UkOaUTz7nvnkRR46qt24Kv1M3sCEHm1WjjrdxaZfyhhVrVvuJ/8B\n3WdNwwvPTOQLdJk0N/fVl+nozf5V9KE1DOUFNgq/NVGTLkUUlT+cUFm/Uj3+0f5k\nCFcD/K8SnaukzCRXhXD82j+/6dtAaPyjkEHNKHYofwzvQPtYTdpwYS0Dts9hXmyb\n3g7DU2ewyPevWZSoZ4n9wKTpQ6GamGTdJEtu40VIJGIxQ/yBQYz7LXm1X/EriDMJ\nex4hoTC18Gvtjyo7uFA+nINcwqpG/V0W7D3+CNSdaQKBgQDNVXwE7rn9DOhlzHMm\nFkcHe7p3tgcCts/NC775Kyw+So9+qFRQRdQ1Hminx5w4a3+fR6eVGMf57Xl2AkGy\nat8F6KXn3iXT4qml/J/arkgGsd9YrUBAtN173owdEoNJP1wE4ho7Q58Y+U+zQ51D\n/SZYpRhjiAK+1eJpJWJRfXzGiQKBgQDHnwnfu5Vh4Of1BMTuhVK6O3lxolYYF8w3\nx+H65EN2EjXlG/EYw3RsG9QHhTqF6JVM6vyqFZjRfcuz9DET5fcn0DKvR3D7iVeU\ne/2pCHE+uvEZ9Su0gHZiXT68c/1c+9C7nXqexW16uTm5G7WOZMpEEaAIdRp/toEu\n5AWKDS8fPQKBgDxYvFtCwhyx93c7sDfoYjW70mCueb79dXMg2Z6nZphkF2o1FJqG\n+0glSMLOsoYOafKo/4KdRuCYP5NENIS4ThWRe3j63Ak623syFNUTVY3KJwcL3A9o\nWJO4I1vD/hu/6E5zGRyD0jVnyFm6LHU36FYzJ0jRR2VIvQMD/rJOfCZpAoGBAKnf\nIu3rmXGTjJCrIFLBzeaBGhWjSZRzG+wUArAYc3gUgxyWrJKgMYCWJdbIf0bY58Ru\n69hpTIRpgmF+2gzO04Zj293g87p547eN1Ax2DLiPKQEn66tM7nFCXFLOebsY50Xg\n+yoFY+bdnMtzUwr7pkxKm17XGFe6HTCkBjq2gXUZAoGBAJ8KzbfnHGmmpimolnL+\nTPXNJP6QSwQi92UgIIbh8/BsFyW+5X/3cHhmFo5IlHlOktbT+zru6jv3EG6omqNQ\nWRlFVR6p6VwWjqzTZbR+dZUbRpSAk3jFZ+HIIkhfYpupMYySUf6UpQL10hQWrlYT\n87vTIKToXQtdQtTCFKSYkryf\n-----END PRIVATE KEY-----",
        "url": "%{MOCK_URL}",
        "tokenUrl": "%{MOCK_URL}/oauth2/token"
      }
      """
    And the mock service responds to POST "/oauth2/token" with status 200 and body:
      """
      {"access_token": "svc-token", "expires_in": 3600}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | gmail         |
    And the connections "Start -> Node"
    And the node "Node" uses the "googleApi" credential "Service Account"

  # ---- message: send -------------------------------------------------------

  Scenario: Send a plain text message
    Given the mock service responds to POST "/gmail/v1/users/me/messages/send" with status 200 and body:
      """
      {"id": "m1", "threadId": "t1", "labelIds": ["SENT"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "send", "sendTo": "bob@example.com", "subject": "Hello World!", "emailType": "text", "message": "Hi Bob", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/send" had the header "authorization" equal to "Bearer svc-token"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "To: <bob@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Subject: Hello World!"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Hi Bob"
    And the node "Node" outputs:
      """
      [{"id": "m1", "threadId": "t1", "labelIds": ["SENT"]}]
      """

  Scenario: Send an HTML message with CC, BCC, Send Replies To, a sender name and the n8n attribution
    Given the mock service responds to GET "/gmail/v1/users/me/profile" with status 200 and body:
      """
      {"emailAddress": "me@example.com"}
      """
    And the mock service responds to POST "/gmail/v1/users/me/messages/send" with status 200 and body:
      """
      {"id": "m2", "threadId": "t2", "labelIds": ["SENT"]}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "serviceAccount", "resource": "message",
        "operation": "send",
        "sendTo": "bob@example.com",
        "subject": "Hi",
        "emailType": "html",
        "message": "<p>Hello</p>",
        "options": {
          "ccList": "cc@example.com",
          "bccList": "bcc@example.com",
          "replyTo": "reply@example.com",
          "senderName": "Nathan",
          "appendAttribution": true
        }
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "From: Nathan <me@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Cc: <cc@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Bcc: <bcc@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Reply-To: <reply@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "This email was sent automatically with"

  Scenario: Send a message with a binary attachment
    Given the mock service responds to POST "/gmail/v1/users/me/messages/send" with status 200 and body:
      """
      {"id": "m3", "threadId": "t3", "labelIds": ["SENT"]}
      """
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "file contents" and mime type "text/plain"
    And the node "Node" has parameters:
      """
      {
        "authentication": "serviceAccount", "resource": "message",
        "operation": "send",
        "sendTo": "bob@example.com",
        "subject": "With attachment",
        "emailType": "text",
        "message": "See attached",
        "options": {"attachmentsUi": {"attachmentsBinary": [{"property": "data"}]}}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Content-Disposition: attachment"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "data.txt"

  # ---- message: reply -------------------------------------------------------

  Scenario: Reply to a message addresses the original sender and keeps the thread
    Given the mock service responds to GET "/gmail/v1/users/me/messages/orig1" with status 200 and body:
      """
      {
        "id": "orig1",
        "threadId": "t1",
        "payload": {
          "headers": [
            {"name": "Subject", "value": "Original subject"},
            {"name": "Message-ID", "value": "<orig@mail.gmail.com>"},
            {"name": "From", "value": "Alice <alice@example.com>"},
            {"name": "To", "value": "me@example.com"}
          ]
        }
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/profile" with status 200 and body:
      """
      {"emailAddress": "me@example.com"}
      """
    And the mock service responds to POST "/gmail/v1/users/me/messages/send" with status 200 and body:
      """
      {"id": "m4", "threadId": "t1", "labelIds": ["SENT"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "reply", "messageId": "orig1", "emailType": "text", "message": "Thanks!", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "To: Alice <alice@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "Subject: Original subject"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "In-Reply-To: <orig@mail.gmail.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "References: <orig@mail.gmail.com>"
    And the last request to "/gmail/v1/users/me/messages/send" had a JSON body matching:
      """
      {"threadId": "t1"}
      """

  # ---- message: get ----------------------------------------------------------

  Scenario: Get a message (simplified) flattens headers and resolves labels, decoding the snippet
    Given the mock service responds to GET "/gmail/v1/users/me/messages/m1" with status 200 and body:
      """
      {
        "id": "m1",
        "threadId": "t1",
        "labelIds": ["INBOX", "UNREAD"],
        "snippet": "Hello &amp; welcome",
        "payload": {"headers": [{"name": "Subject", "value": "Hi"}, {"name": "From", "value": "a@b.com"}]}
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": [{"id": "INBOX", "name": "Inbox"}, {"id": "UNREAD", "name": "Unread"}]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "get", "messageId": "m1", "simple": true}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m1" had the query parameter "format" equal to "metadata"
    And the node "Node" outputs items matching:
      """
      [{
        "id": "m1",
        "threadId": "t1",
        "Subject": "Hi",
        "From": "a@b.com",
        "snippet": "Hello & welcome",
        "labels": [{"id": "INBOX", "name": "Inbox"}, {"id": "UNREAD", "name": "Unread"}]
      }]
      """

  Scenario: Get a message (not simplified) parses the raw MIME body
    Given the mock service responds to GET "/gmail/v1/users/me/messages/m5" with status 200 and the raw message (id "m5", thread "t5"):
      """
      From: Alice <alice@example.com>
      To: Bob <bob@example.com>
      Subject: Full test
      Content-Type: text/plain; charset=utf-8

      Hello body text.
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "get", "messageId": "m5", "simple": false, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m5" had the query parameter "format" equal to "raw"
    And the node "Node" outputs items matching:
      """
      [{
        "id": "m5",
        "threadId": "t5",
        "subject": "Full test",
        "text": "$contains:Hello body text.",
        "html": false,
        "headers": {"subject": "Subject: Full test"}
      }]
      """

  Scenario: Get a message with Download Attachments extracts the attachment into binary
    Given the mock service responds to GET "/gmail/v1/users/me/messages/m6" with status 200 and the raw message (id "m6", thread "t6"):
      """
      From: Alice <alice@example.com>
      To: Bob <bob@example.com>
      Subject: Has attachment
      Content-Type: multipart/mixed; boundary="BOUNDARY1"

      --BOUNDARY1
      Content-Type: text/plain; charset=utf-8

      See attached.
      --BOUNDARY1
      Content-Type: text/plain
      Content-Disposition: attachment; filename="notes.txt"
      Content-Transfer-Encoding: base64

      SGVsbG8gYXR0YWNobWVudA==
      --BOUNDARY1--
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "get", "messageId": "m6", "simple": false, "options": {"downloadAttachments": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "m6", "text": "$contains:See attached."}]
      """
    And the node "Node" output item 0 has the binary property "attachment_0" with file name "notes.txt"

  # ---- message: getAll -------------------------------------------------------

  Scenario: Get many messages (simplified, not returnAll) applies the limit and fetches each message's metadata
    Given the mock service responds to GET "/gmail/v1/users/me/messages" with status 200 and body:
      """
      {"messages": [{"id": "a1"}, {"id": "a2"}]}
      """
    And the mock service responds to GET "/gmail/v1/users/me/messages/a1" with status 200 and body:
      """
      {"id": "a1", "threadId": "t1", "labelIds": [], "payload": {"headers": [{"name": "Subject", "value": "First"}]}}
      """
    And the mock service responds to GET "/gmail/v1/users/me/messages/a2" with status 200 and body:
      """
      {"id": "a2", "threadId": "t2", "labelIds": [], "payload": {"headers": [{"name": "Subject", "value": "Second"}]}}
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "getAll", "returnAll": false, "limit": 50, "simple": true, "filters": {}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages" had the query parameter "maxResults" equal to "50"
    And the node "Node" outputs items matching:
      """
      [{"id": "a1", "threadId": "t1", "Subject": "First"}, {"id": "a2", "threadId": "t2", "Subject": "Second"}]
      """

  Scenario: Get many messages (returnAll) pages through nextPageToken
    Given the mock service responds to GET "/gmail/v1/users/me/messages" in order with:
      """
      [
        {"status": 200, "body": {"messages": [{"id": "p1"}], "nextPageToken": "page2"}},
        {"status": 200, "body": {"messages": [{"id": "p2"}]}}
      ]
      """
    And the mock service responds to GET "/gmail/v1/users/me/messages/p1" with status 200 and body:
      """
      {"id": "p1", "threadId": "t1", "labelIds": [], "payload": {"headers": []}}
      """
    And the mock service responds to GET "/gmail/v1/users/me/messages/p2" with status 200 and body:
      """
      {"id": "p2", "threadId": "t2", "labelIds": [], "payload": {"headers": []}}
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "getAll", "returnAll": true, "simple": true, "filters": {}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs 2 items
    And the 2nd request to "/gmail/v1/users/me/messages" had the query parameter "pageToken" equal to "page2"

  Scenario: Get many messages with filters builds the Gmail search query
    Given the mock service responds to GET "/gmail/v1/users/me/messages" with status 200 and body:
      """
      {"messages": []}
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the node "Node" has parameters:
      """
      {
        "authentication": "serviceAccount", "resource": "message",
        "operation": "getAll",
        "returnAll": false,
        "limit": 10,
        "simple": true,
        "filters": {"q": "has:attachment", "sender": "alice@example.com", "readStatus": "unread"},
        "options": {}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages" had the query parameter "q" equal to "has:attachment from:alice@example.com is:unread"

  # ---- message: delete / mark / labels ---------------------------------------

  Scenario: Delete a message
    Given the mock service responds to DELETE "/gmail/v1/users/me/messages/m7" with status 200
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "delete", "messageId": "m7"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  Scenario: Mark a message as read removes the UNREAD label
    Given the mock service responds to POST "/gmail/v1/users/me/messages/m8/modify" with status 200 and body:
      """
      {"id": "m8", "labelIds": ["INBOX"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "markAsRead", "messageId": "m8"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m8/modify" had a JSON body matching:
      """
      {"removeLabelIds": ["UNREAD"]}
      """

  Scenario: Mark a message as unread adds the UNREAD label
    Given the mock service responds to POST "/gmail/v1/users/me/messages/m9/modify" with status 200 and body:
      """
      {"id": "m9", "labelIds": ["INBOX", "UNREAD"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "markAsUnread", "messageId": "m9"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m9/modify" had a JSON body matching:
      """
      {"addLabelIds": ["UNREAD"]}
      """

  Scenario: Add labels to a message
    Given the mock service responds to POST "/gmail/v1/users/me/messages/m10/modify" with status 200 and body:
      """
      {"id": "m10", "labelIds": ["IMPORTANT"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "addLabels", "messageId": "m10", "labelIds": ["IMPORTANT"]}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m10/modify" had a JSON body matching:
      """
      {"addLabelIds": ["IMPORTANT"]}
      """

  Scenario: Remove labels from a message
    Given the mock service responds to POST "/gmail/v1/users/me/messages/m11/modify" with status 200 and body:
      """
      {"id": "m11", "labelIds": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "removeLabels", "messageId": "m11", "labelIds": ["IMPORTANT"]}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/m11/modify" had a JSON body matching:
      """
      {"removeLabelIds": ["IMPORTANT"]}
      """

  # ---- draft ------------------------------------------------------------------

  Scenario: Create a draft
    Given the mock service responds to POST "/gmail/v1/users/me/drafts" with status 200 and body:
      """
      {"id": "d1", "message": {"id": "dm1", "threadId": "t1"}}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "draft", "operation": "create", "subject": "Draft subject", "emailType": "text", "message": "Draft body", "options": {"sendTo": "bob@example.com"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/drafts" has a decoded raw body containing "To: <bob@example.com>"
    And the last request to "/gmail/v1/users/me/drafts" has a decoded raw body containing "Subject: Draft subject"
    And the node "Node" outputs:
      """
      [{"id": "d1", "message": {"id": "dm1", "threadId": "t1"}}]
      """

  Scenario: Create a draft attached to an existing thread copies the last message's Message-ID
    Given the mock service responds to GET "/gmail/v1/users/me/threads/t9" with status 200 and body:
      """
      {
        "id": "t9",
        "messages": [
          {"id": "x1", "payload": {"headers": [{"name": "Message-ID", "value": "<old@mail.gmail.com>"}]}},
          {"id": "x2", "payload": {"headers": [{"name": "Message-ID", "value": "<last@mail.gmail.com>"}]}}
        ]
      }
      """
    And the mock service responds to POST "/gmail/v1/users/me/drafts" with status 200 and body:
      """
      {"id": "d2", "message": {"id": "dm2", "threadId": "t9"}}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "draft", "operation": "create", "subject": "Re: thread", "emailType": "text", "message": "Following up", "options": {"threadId": "t9"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/drafts" has a decoded raw body containing "In-Reply-To: <last@mail.gmail.com>"
    And the last request to "/gmail/v1/users/me/drafts" has a decoded raw body containing "References: <last@mail.gmail.com>"
    And the last request to "/gmail/v1/users/me/drafts" had a JSON body matching:
      """
      {"message": {"threadId": "t9"}}
      """

  Scenario: Get a draft parses the raw message and renames the message ID
    Given the mock service responds to GET "/gmail/v1/users/me/drafts/d3" with status 200 and the raw draft (id "d3", message id "innerm3", thread "t3"):
      """
      From: Alice <alice@example.com>
      Subject: Draft
      Content-Type: text/plain

      Draft body
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "draft", "operation": "get", "messageId": "d3", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "d3", "messageId": "innerm3", "threadId": "t3", "subject": "Draft", "text": "$contains:Draft body"}]
      """

  Scenario: Get many drafts
    Given the mock service responds to GET "/gmail/v1/users/me/drafts" with status 200 and body:
      """
      {"drafts": [{"id": "d4"}]}
      """
    And the mock service responds to GET "/gmail/v1/users/me/drafts/d4" with status 200 and the raw draft (id "d4", message id "innerm4", thread "t4"):
      """
      Subject: Hello
      Content-Type: text/plain

      Hi
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "draft", "operation": "getAll", "returnAll": false, "limit": 50, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "d4", "messageId": "innerm4", "subject": "Hello"}]
      """

  Scenario: Delete a draft
    Given the mock service responds to DELETE "/gmail/v1/users/me/drafts/d5" with status 200
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "draft", "operation": "delete", "messageId": "d5"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  # ---- label ------------------------------------------------------------------

  Scenario: Create a label
    Given the mock service responds to POST "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"id": "Label_1", "name": "invoices", "labelListVisibility": "labelShow", "messageListVisibility": "show"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "label", "operation": "create", "name": "invoices", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "Label_1", "name": "invoices"}]
      """

  Scenario: Get a label
    Given the mock service responds to GET "/gmail/v1/users/me/labels/Label_1" with status 200 and body:
      """
      {"id": "Label_1", "name": "invoices"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "label", "operation": "get", "labelId": "Label_1"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "Label_1", "name": "invoices"}]
      """

  Scenario: Get many labels
    Given the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": [{"id": "INBOX", "name": "Inbox"}, {"id": "Label_1", "name": "invoices"}]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "label", "operation": "getAll", "returnAll": true}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "INBOX", "name": "Inbox"}, {"id": "Label_1", "name": "invoices"}]
      """

  Scenario: Delete a label
    Given the mock service responds to DELETE "/gmail/v1/users/me/labels/Label_1" with status 200
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "label", "operation": "delete", "labelId": "Label_1"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  # ---- thread -----------------------------------------------------------------

  Scenario: Get a thread (simplified, default) returns the whole thread with simplified messages
    Given the mock service responds to GET "/gmail/v1/users/me/threads/t1" with status 200 and body:
      """
      {
        "id": "t1",
        "historyId": "h1",
        "messages": [
          {"id": "m1", "threadId": "t1", "labelIds": ["INBOX"], "payload": {"headers": [{"name": "Subject", "value": "Hi"}]}}
        ]
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": [{"id": "INBOX", "name": "Inbox"}]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "get", "threadId": "t1", "simple": true, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "t1", "historyId": "h1", "messages": [{"id": "m1", "threadId": "t1", "Subject": "Hi", "labels": [{"id": "INBOX", "name": "Inbox"}]}]}]
      """

  Scenario: Get a thread with Return Only Messages outputs each message as a separate item
    Given the mock service responds to GET "/gmail/v1/users/me/threads/t1" with status 200 and body:
      """
      {
        "id": "t1",
        "messages": [
          {"id": "m1", "threadId": "t1", "labelIds": [], "payload": {"headers": [{"name": "Subject", "value": "First"}]}},
          {"id": "m2", "threadId": "t1", "labelIds": [], "payload": {"headers": [{"name": "Subject", "value": "Second"}]}}
        ]
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "get", "threadId": "t1", "simple": true, "options": {"returnOnlyMessages": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs 2 items
    And the node "Node" outputs items matching:
      """
      [{"id": "m1", "threadId": "t1", "Subject": "First"}, {"id": "m2", "threadId": "t1", "Subject": "Second"}]
      """

  Scenario: Get many threads
    Given the mock service responds to GET "/gmail/v1/users/me/threads" with status 200 and body:
      """
      {"threads": [{"id": "t1", "snippet": "Hi"}, {"id": "t2", "snippet": "Yo"}]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "getAll", "returnAll": false, "limit": 50, "filters": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs items matching:
      """
      [{"id": "t1"}, {"id": "t2"}]
      """

  Scenario: Delete a thread
    Given the mock service responds to DELETE "/gmail/v1/users/me/threads/t1" with status 200
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "delete", "threadId": "t1"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  Scenario: Trash a thread
    Given the mock service responds to POST "/gmail/v1/users/me/threads/t1/trash" with status 200 and body:
      """
      {"id": "t1", "labelIds": ["TRASH"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "trash", "threadId": "t1"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "t1", "labelIds": ["TRASH"]}]
      """

  Scenario: Untrash a thread
    Given the mock service responds to POST "/gmail/v1/users/me/threads/t1/untrash" with status 200 and body:
      """
      {"id": "t1", "labelIds": ["INBOX"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "untrash", "threadId": "t1"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "t1", "labelIds": ["INBOX"]}]
      """

  Scenario: Add labels to a thread
    Given the mock service responds to POST "/gmail/v1/users/me/threads/t1/modify" with status 200 and body:
      """
      {"id": "t1", "labelIds": ["IMPORTANT"]}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "addLabels", "threadId": "t1", "labelIds": ["IMPORTANT"]}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/threads/t1/modify" had a JSON body matching:
      """
      {"addLabelIds": ["IMPORTANT"]}
      """

  Scenario: Remove labels from a thread
    Given the mock service responds to POST "/gmail/v1/users/me/threads/t1/modify" with status 200 and body:
      """
      {"id": "t1", "labelIds": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "removeLabels", "threadId": "t1", "labelIds": ["IMPORTANT"]}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/threads/t1/modify" had a JSON body matching:
      """
      {"removeLabelIds": ["IMPORTANT"]}
      """

  Scenario: Reply to a thread
    Given the mock service responds to GET "/gmail/v1/users/me/messages/orig2" with status 200 and body:
      """
      {
        "id": "orig2",
        "threadId": "t1",
        "payload": {
          "headers": [
            {"name": "Subject", "value": "Thread subject"},
            {"name": "Message-ID", "value": "<orig2@mail.gmail.com>"},
            {"name": "From", "value": "Alice <alice@example.com>"},
            {"name": "To", "value": "me@example.com"}
          ]
        }
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/profile" with status 200 and body:
      """
      {"emailAddress": "me@example.com"}
      """
    And the mock service responds to POST "/gmail/v1/users/me/messages/send" with status 200 and body:
      """
      {"id": "m12", "threadId": "t1"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "thread", "operation": "reply", "messageId": "orig2", "emailType": "text", "message": "On it", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "To: Alice <alice@example.com>"
    And the last request to "/gmail/v1/users/me/messages/send" has a decoded raw body containing "In-Reply-To: <orig2@mail.gmail.com>"

  # ---- auth / errors / edge cases ---------------------------------------------

  Scenario: A missing Gmail credential fails with a clear message
    Given the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "label", "operation": "getAll", "returnAll": true}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: Clear OAuth2 authentication connects with a bearer token
    Given the credential "OAuth Gmail" of type "gmailOAuth2" with the data:
      """
      {"url": "%{MOCK_URL}", "oauthTokenData": {"access_token": "gmail-oauth-token"}}
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "label", "operation": "getAll", "returnAll": true}
      """
    And the node "Node" uses the "gmailOAuth2" credential "OAuth Gmail"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/gmail/v1/users/me/labels" had the header "authorization" equal to "Bearer gmail-oauth-token"

  Scenario: A Gmail OAuth2 credential that was never connected fails with a clear message
    Given the credential "Unconnected" of type "gmailOAuth2" with the data:
      """
      {"url": "%{MOCK_URL}"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "label", "operation": "getAll", "returnAll": true}
      """
    And the node "Node" uses the "gmailOAuth2" credential "Unconnected"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not connected"

  Scenario: A 401 triggers one OAuth2 token refresh and a retry
    Given the credential "OAuth Gmail" of type "gmailOAuth2" with the data:
      """
      {
        "url": "%{MOCK_URL}",
        "accessTokenUrl": "%{MOCK_URL}/oauth2/refresh",
        "clientId": "client-1",
        "clientSecret": "secret-1",
        "oauthTokenData": {"access_token": "expired-token", "refresh_token": "refresh-1"}
      }
      """
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 401 the first 1 times
    And the mock service responds to GET "/gmail/v1/users/me/labels" with status 200 and body:
      """
      {"labels": []}
      """
    And the mock service responds to POST "/oauth2/refresh" with status 200 and body:
      """
      {"access_token": "fresh-token", "refresh_token": "refresh-2"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "label", "operation": "getAll", "returnAll": true}
      """
    And the node "Node" uses the "gmailOAuth2" credential "OAuth Gmail"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/gmail/v1/users/me/labels"
    And the mock service received 1 request to "/oauth2/refresh"
    And the last request to "/gmail/v1/users/me/labels" had the header "authorization" equal to "Bearer fresh-token"

  Scenario: A 404 from Gmail becomes a clear error and the token never leaks
    Given the mock service responds to GET "/gmail/v1/users/me/messages/missing" with status 404 and body:
      """
      {"error": {"code": 404, "message": "Requested entity was not found."}}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "get", "messageId": "missing", "simple": true, "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Message not found"
    And the execution data does not contain "svc-token"

  Scenario: A 409 creating a duplicate label becomes a clear error
    Given the mock service responds to POST "/gmail/v1/users/me/labels" with status 409 and body:
      """
      {"error": {"code": 409, "message": "Label name exists or conflicts"}}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "label", "operation": "create", "name": "invoices", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Label name exists already"

  Scenario: continueOnFail turns a Gmail error into an error item instead of failing the run
    Given the mock service responds to GET "/gmail/v1/users/me/messages/m1" with status 500
    And the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "get", "messageId": "m1", "simple": true, "options": {}}
      """
    And the node "Node" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"error": "$contains:status code 500"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "settings", "operation": "getAll"}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given the node "Node" has parameters:
      """
      {"authentication": "serviceAccount", "resource": "message", "operation": "sendAndWait"}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
