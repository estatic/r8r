@spec-6.6 @phase-4 @node-telegram
Feature: Telegram node
  Consumes the Telegram Bot API (v1/1.1/1.2, as the n8n 2.35.7 editor
  creates the node) against the `telegramApi` credential. Telegram Trigger
  and `sendAndWait` are out of scope; anything unimplemented fails with a
  clear message.

  Background:
    Given a mock HTTP service
    And the credential "Bot" of type "telegramApi" with the data:
      """
      {"accessToken": "test_bot_token", "baseUrl": "%{MOCK_URL}"}
      """

  # ---- chat --------------------------------------------------------------

  Scenario: Getting a chat
    Given the mock service responds to POST "/bottest_bot_token/getChat" with status 200 and body:
      """
      {"ok": true, "result": {"id": 123, "type": "private", "first_name": "Ada"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Get   | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "chat", "operation": "get", "chatId": "123"}
      """
    And the node "Get" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/getChat" had a JSON body matching:
      """
      {"chat_id": "123"}
      """
    And the node "Get" outputs:
      """
      [{"ok": true, "result": {"id": 123, "type": "private", "first_name": "Ada"}}]
      """

  Scenario: Getting a chat's administrators explodes the result array into one item per administrator
    Given the mock service responds to POST "/bottest_bot_token/getChatAdministrators" with status 200 and body:
      """
      {"ok": true, "result": [
        {"user": {"id": 1, "first_name": "Alice"}, "status": "creator"},
        {"user": {"id": 2, "first_name": "Bob"}, "status": "administrator"}
      ]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Admin | telegram      |
    And the node "Admin" has parameters:
      """
      {"resource": "chat", "operation": "administrators", "chatId": "123"}
      """
    And the node "Admin" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Admin"
    When I execute the workflow
    Then the execution succeeds
    And the node "Admin" outputs:
      """
      [
        {"user": {"id": 1, "first_name": "Alice"}, "status": "creator"},
        {"user": {"id": 2, "first_name": "Bob"}, "status": "administrator"}
      ]
      """

  Scenario: Getting a chat member
    Given the mock service responds to POST "/bottest_bot_token/getChatMember" with status 200 and body:
      """
      {"ok": true, "result": {"user": {"id": 42}, "status": "member"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Member | telegram      |
    And the node "Member" has parameters:
      """
      {"resource": "chat", "operation": "member", "chatId": "123", "userId": "42"}
      """
    And the node "Member" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Member"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/getChatMember" had a JSON body matching:
      """
      {"chat_id": "123", "user_id": "42"}
      """
    And the node "Member" outputs:
      """
      [{"ok": true, "result": {"user": {"id": 42}, "status": "member"}}]
      """

  Scenario: Leaving a chat
    Given the mock service responds to POST "/bottest_bot_token/leaveChat" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Leave | telegram      |
    And the node "Leave" has parameters:
      """
      {"resource": "chat", "operation": "leave", "chatId": "123"}
      """
    And the node "Leave" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Leave"
    When I execute the workflow
    Then the execution succeeds
    And the node "Leave" outputs:
      """
      [{"ok": true, "result": true}]
      """

  Scenario: Setting a chat's description
    Given the mock service responds to POST "/bottest_bot_token/setChatDescription" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Desc  | telegram      |
    And the node "Desc" has parameters:
      """
      {"resource": "chat", "operation": "setDescription", "chatId": "123", "description": "A test chat"}
      """
    And the node "Desc" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Desc"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/setChatDescription" had a JSON body matching:
      """
      {"chat_id": "123", "description": "A test chat"}
      """

  Scenario: Setting a chat's title
    Given the mock service responds to POST "/bottest_bot_token/setChatTitle" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Title | telegram      |
    And the node "Title" has parameters:
      """
      {"resource": "chat", "operation": "setTitle", "chatId": "123", "title": "New title"}
      """
    And the node "Title" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Title"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/setChatTitle" had a JSON body matching:
      """
      {"chat_id": "123", "title": "New title"}
      """

  # ---- callback ------------------------------------------------------------

  Scenario: Answering a callback query
    Given the mock service responds to POST "/bottest_bot_token/answerCallbackQuery" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Answer | telegram      |
    And the node "Answer" has parameters:
      """
      {"resource": "callback", "operation": "answerQuery", "queryId": "q1", "additionalFields": {"text": "Done!", "show_alert": true}}
      """
    And the node "Answer" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Answer"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/answerCallbackQuery" had a JSON body matching:
      """
      {"callback_query_id": "q1", "text": "Done!", "show_alert": true}
      """
    And the node "Answer" outputs:
      """
      [{"ok": true, "result": true}]
      """

  Scenario: Answering an inline query
    Given the mock service responds to POST "/bottest_bot_token/answerInlineQuery" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Answer | telegram      |
    And the node "Answer" has parameters:
      """
      {
        "resource": "callback",
        "operation": "answerInlineQuery",
        "queryId": "q1",
        "results": [{"type": "article", "id": "1", "title": "Result 1"}],
        "additionalFields": {"cache_time": 10}
      }
      """
    And the node "Answer" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Answer"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/answerInlineQuery" had a JSON body matching:
      """
      {"inline_query_id": "q1", "results": [{"type": "article", "id": "1", "title": "Result 1"}], "cache_time": 10}
      """

  # ---- file ----------------------------------------------------------------

  Scenario: Getting a file downloads it as binary data
    Given the mock service responds to POST "/bottest_bot_token/getFile" with status 200 and body:
      """
      {"ok": true, "result": {"file_id": "F1", "file_path": "documents/file_1.txt"}}
      """
    And the mock service responds to GET "/file/bottest_bot_token/documents/file_1.txt" with status 200 and body:
      """
      hello world
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "file", "operation": "get", "fileId": "F1", "download": true}
      """
    And the node "Get" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Get": [
                {
                  "data": {
                    "main": [[
                      {
                        "json": {"ok": true, "result": {"file_id": "F1", "file_path": "documents/file_1.txt"}},
                        "binary": {"data": {"data": "aGVsbG8gd29ybGQ=", "fileName": "file_1.txt", "mimeType": "text/plain"}}
                      }
                    ]]
                  }
                }
              ]
            }
          }
        }
      }
      """

  Scenario: Getting a file without downloading returns the raw API response
    Given the mock service responds to POST "/bottest_bot_token/getFile" with status 200 and body:
      """
      {"ok": true, "result": {"file_id": "F1", "file_path": "documents/file_1.txt"}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "file", "operation": "get", "fileId": "F1", "download": false}
      """
    And the node "Get" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/bottest_bot_token/getFile"
    And the mock service received 0 requests to "/file/bottest_bot_token/documents/file_1.txt"
    And the node "Get" outputs:
      """
      [{"ok": true, "result": {"file_id": "F1", "file_path": "documents/file_1.txt"}}]
      """

  # ---- message: sendMessage (attribution / preview defaults) ---------------

  Scenario: Sending a message appends the n8n attribution and disables the link preview by default
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 1, "text": "hello"}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendMessage", "chatId": "123", "text": "hello", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendMessage" had a JSON body matching:
      """
      {
        "chat_id": "123",
        "text": "hello\n\n_This message was sent automatically with _[n8n](https://n8n.io/?utm_source=n8n-internal&utm_medium=powered_by&utm_campaign=n8n-nodes-base.telegram)",
        "disable_web_page_preview": true,
        "parse_mode": "Markdown"
      }
      """
    And the node "Send" outputs:
      """
      [{"ok": true, "result": {"message_id": 1, "text": "hello"}}]
      """

  Scenario: Sending a message can opt out of attribution and override the link preview default
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 1}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {
        "resource": "message",
        "operation": "sendMessage",
        "chatId": "123",
        "text": "hi",
        "additionalFields": {"appendAttribution": false, "disable_web_page_preview": false}
      }
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendMessage" had a JSON body matching:
      """
      {"chat_id": "123", "text": "hi", "disable_web_page_preview": false, "parse_mode": "Markdown"}
      """

  Scenario: On typeVersion 1.0, the link preview default only kicks in when the text has no URL, and attribution is not defaulted on
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 1}}
      """
    And a workflow with nodes:
      | name | type          | typeVersion |
      | Start| manualTrigger |             |
      | Send | telegram      | 1           |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendMessage", "chatId": "123", "text": "Check https://example.com", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendMessage" had a JSON body matching:
      """
      {"chat_id": "123", "text": "Check https://example.com", "parse_mode": "Markdown"}
      """

  Scenario: Sending a message with an inline keyboard
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 1}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {
        "resource": "message",
        "operation": "sendMessage",
        "chatId": "123",
        "text": "Pick one",
        "additionalFields": {"appendAttribution": false},
        "replyMarkup": "inlineKeyboard",
        "inlineKeyboard": {
          "rows": [
            {"row": {"buttons": [
              {"text": "Yes", "additionalFields": {"callback_data": "yes"}},
              {"text": "No", "additionalFields": {"callback_data": "no"}}
            ]}}
          ]
        }
      }
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendMessage" had a JSON body matching:
      """
      {
        "chat_id": "123",
        "text": "Pick one",
        "reply_markup": {"inline_keyboard": [[
          {"text": "Yes", "callback_data": "yes"},
          {"text": "No", "callback_data": "no"}
        ]]}
      }
      """

  # ---- message: media sends (URL/file_id) -----------------------------------

  Scenario: Sending a photo by URL
    Given the mock service responds to POST "/bottest_bot_token/sendPhoto" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 2}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {
        "resource": "message",
        "operation": "sendPhoto",
        "chatId": "123",
        "binaryData": false,
        "file": "https://example.com/photo.jpg",
        "additionalFields": {"caption": "A photo"}
      }
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendPhoto" had a JSON body matching:
      """
      {"chat_id": "123", "photo": "https://example.com/photo.jpg", "caption": "A photo"}
      """

  Scenario: Sending an audio file by file_id
    Given the mock service responds to POST "/bottest_bot_token/sendAudio" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 3}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendAudio", "chatId": "123", "binaryData": false, "file": "AgADBAAD", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendAudio" had a JSON body matching:
      """
      {"chat_id": "123", "audio": "AgADBAAD"}
      """

  Scenario: Sending a video by URL
    Given the mock service responds to POST "/bottest_bot_token/sendVideo" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 4}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendVideo", "chatId": "123", "binaryData": false, "file": "https://example.com/clip.mp4", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendVideo" had a JSON body matching:
      """
      {"chat_id": "123", "video": "https://example.com/clip.mp4"}
      """

  Scenario: Sending an animation by URL
    Given the mock service responds to POST "/bottest_bot_token/sendAnimation" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 5}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendAnimation", "chatId": "123", "binaryData": false, "file": "https://example.com/clip.gif", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendAnimation" had a JSON body matching:
      """
      {"chat_id": "123", "animation": "https://example.com/clip.gif"}
      """

  Scenario: Sending a sticker by file_id
    Given the mock service responds to POST "/bottest_bot_token/sendSticker" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 6}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendSticker", "chatId": "123", "binaryData": false, "file": "CAACAgIA", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendSticker" had a JSON body matching:
      """
      {"chat_id": "123", "sticker": "CAACAgIA"}
      """

  Scenario: Sending a document from binary input data uploads it as multipart/form-data
    Given the mock service responds to POST "/bottest_bot_token/sendDocument" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 7}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {
        "resource": "message",
        "operation": "sendDocument",
        "chatId": "123",
        "binaryData": true,
        "binaryPropertyName": "data",
        "additionalFields": {"caption": "report"}
      }
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "hello file" and mime type "text/plain"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendDocument" had the multipart field "chat_id" equal to "123"
    And the last request to "/bottest_bot_token/sendDocument" had the multipart field "caption" equal to "report"
    And the last request to "/bottest_bot_token/sendDocument" had the multipart field "disable_notification" equal to "false"
    And the last request to "/bottest_bot_token/sendDocument" had a multipart file field "document" with filename "data.txt"
    And the node "Send" outputs:
      """
      [{"ok": true, "result": {"message_id": 7}}]
      """

  Scenario: A binary send with no file name available fails with a clear error
    Given a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendPhoto", "chatId": "123", "binaryData": true, "binaryPropertyName": "data", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "hello" and mime type "application/octet-stream" and no file name
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "File name is needed to sendPhoto"

  # ---- message: everything else ---------------------------------------------

  Scenario: Sending a location
    Given the mock service responds to POST "/bottest_bot_token/sendLocation" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 8}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendLocation", "chatId": "123", "latitude": 51.5, "longitude": -0.12, "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendLocation" had a JSON body matching:
      """
      {"chat_id": "123", "latitude": 51.5, "longitude": -0.12}
      """

  Scenario: Sending a media group flattens each item's additional fields
    Given the mock service responds to POST "/bottest_bot_token/sendMediaGroup" with status 200 and body:
      """
      {"ok": true, "result": [{"message_id": 9}, {"message_id": 10}]}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {
        "resource": "message",
        "operation": "sendMediaGroup",
        "chatId": "123",
        "additionalFields": {},
        "media": {
          "media": [
            {"type": "photo", "media": "https://example.com/1.jpg", "additionalFields": {"caption": "first"}},
            {"type": "photo", "media": "https://example.com/2.jpg", "additionalFields": {"caption": "second"}}
          ]
        }
      }
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendMediaGroup" had a JSON body matching:
      """
      {
        "chat_id": "123",
        "media": [
          {"type": "photo", "media": "https://example.com/1.jpg", "caption": "first"},
          {"type": "photo", "media": "https://example.com/2.jpg", "caption": "second"}
        ]
      }
      """

  Scenario: Sending a chat action
    Given the mock service responds to POST "/bottest_bot_token/sendChatAction" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendChatAction", "chatId": "123", "action": "typing"}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/sendChatAction" had a JSON body matching:
      """
      {"chat_id": "123", "action": "typing"}
      """

  Scenario: Editing a message's text by chat and message ID
    Given the mock service responds to POST "/bottest_bot_token/editMessageText" with status 200 and body:
      """
      {"ok": true, "result": {"message_id": 1}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Edit | telegram      |
    And the node "Edit" has parameters:
      """
      {
        "resource": "message",
        "operation": "editMessageText",
        "messageType": "message",
        "chatId": "123",
        "messageId": "1",
        "text": "updated",
        "additionalFields": {"parse_mode": "HTML"}
      }
      """
    And the node "Edit" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Edit"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/editMessageText" had a JSON body matching:
      """
      {"chat_id": "123", "message_id": "1", "text": "updated", "parse_mode": "HTML"}
      """

  Scenario: Editing a message's text by inline message ID
    Given the mock service responds to POST "/bottest_bot_token/editMessageText" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Edit | telegram      |
    And the node "Edit" has parameters:
      """
      {
        "resource": "message",
        "operation": "editMessageText",
        "messageType": "inlineMessage",
        "inlineMessageId": "abc123",
        "text": "updated",
        "additionalFields": {}
      }
      """
    And the node "Edit" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Edit"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/editMessageText" had a JSON body matching:
      """
      {"inline_message_id": "abc123", "text": "updated"}
      """

  Scenario: Deleting a message
    Given the mock service responds to POST "/bottest_bot_token/deleteMessage" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | telegram      |
    And the node "Delete" has parameters:
      """
      {"resource": "message", "operation": "deleteMessage", "chatId": "123", "messageId": "1"}
      """
    And the node "Delete" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/deleteMessage" had a JSON body matching:
      """
      {"chat_id": "123", "message_id": "1"}
      """

  Scenario: Pinning a chat message with a notification
    Given the mock service responds to POST "/bottest_bot_token/pinChatMessage" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Pin  | telegram      |
    And the node "Pin" has parameters:
      """
      {"resource": "message", "operation": "pinChatMessage", "chatId": "123", "messageId": "1", "additionalFields": {"disable_notification": true}}
      """
    And the node "Pin" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Pin"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/pinChatMessage" had a JSON body matching:
      """
      {"chat_id": "123", "message_id": "1", "disable_notification": true}
      """

  Scenario: Unpinning a chat message
    Given the mock service responds to POST "/bottest_bot_token/unpinChatMessage" with status 200 and body:
      """
      {"ok": true, "result": true}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Unpin | telegram      |
    And the node "Unpin" has parameters:
      """
      {"resource": "message", "operation": "unpinChatMessage", "chatId": "123", "messageId": "1"}
      """
    And the node "Unpin" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Unpin"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/bottest_bot_token/unpinChatMessage" had a JSON body matching:
      """
      {"chat_id": "123", "message_id": "1"}
      """

  # ---- errors, auth, continueOnFail, and unsupported operations ------------

  Scenario: A 400 from Telegram maps to n8n's bad-request message
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 400 and body:
      """
      {"ok": false, "error_code": 400, "description": "Bad Request: chat not found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendMessage", "chatId": "999", "text": "hi", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Bad request - please check your parameters"

  Scenario: A 401 from Telegram maps to n8n's authorization-failed message
    Given the mock service responds to POST "/bottest_bot_token/getChat" with status 401 and body:
      """
      {"ok": false, "error_code": 401, "description": "Unauthorized"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "chat", "operation": "get", "chatId": "123"}
      """
    And the node "Get" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 429 from Telegram maps to n8n's rate-limit message
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 429 and body:
      """
      {"ok": false, "error_code": 429, "description": "Too Many Requests: retry after 30"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendMessage", "chatId": "123", "text": "hi", "additionalFields": {}}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The service is receiving too many requests from you"

  Scenario: A missing Telegram credential fails with a clear message
    Given a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "chat", "operation": "get", "chatId": "123"}
      """
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: A Telegram credential with a blank access token fails with a clear message
    Given the credential "Blank" of type "telegramApi" with the data:
      """
      {"accessToken": "", "baseUrl": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "chat", "operation": "get", "chatId": "123"}
      """
    And the node "Get" uses the "telegramApi" credential "Blank"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Telegram credentials are not set"

  Scenario: The bot token never appears in the execution data
    Given the mock service responds to POST "/bottest_bot_token/getChat" with status 401 and body:
      """
      {"ok": false, "error_code": 401, "description": "Unauthorized"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | telegram      |
    And the node "Get" has parameters:
      """
      {"resource": "chat", "operation": "get", "chatId": "123"}
      """
    And the node "Get" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "test_bot_token"

  Scenario: continueOnFail turns a Telegram error into an error item, preferring the API's own description
    Given the mock service responds to POST "/bottest_bot_token/sendMessage" with status 400 and body:
      """
      {"ok": false, "error_code": 400, "description": "Bad Request: chat not found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Send | telegram      |
    And the node "Send" has parameters:
      """
      {"resource": "message", "operation": "sendMessage", "chatId": "999", "text": "hi", "additionalFields": {}}
      """
    And the node "Send" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Send" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs:
      """
      [{"error": "Bad Request: chat not found"}]
      """

  Scenario: An unimplemented operation returns a clear "not supported" message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Draft | telegram      |
    And the node "Draft" has parameters:
      """
      {"resource": "message", "operation": "sendMessageDraft", "chatId": "123"}
      """
    And the node "Draft" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Draft"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Info  | telegram      |
    And the node "Info" has parameters:
      """
      {"resource": "bot", "operation": "info"}
      """
    And the node "Info" uses the "telegramApi" credential "Bot"
    And the connections "Start -> Info"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
