@spec-6.6 @phase-4 @node-email @requires-mailpit
Feature: Send Email node (SMTP)
  Sends mail over SMTP through the `smtp` credential, matching n8n's
  `EmailSend` node (`n8n-nodes-base.emailSend`) v2/v2.1. `sendAndWait`
  (the v2.1 second operation) is out of scope and returns a clear
  "not supported natively yet" error. Scenarios run in parallel against a
  shared Mailpit instance, so each uses a unique subject and looks its
  message up by subject instead of clearing the mailbox.

  Scenario: A plain text email is delivered
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "text-format@r8r.test",
        "subject": "r8r-bdd-send-email text-format",
        "emailFormat": "text",
        "text": "Plain text body",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email text-format"
    And the Mailpit message with subject "r8r-bdd-send-email text-format" matches:
      """
      {
        "From": {"Address": "sender@r8r.test"},
        "To": [{"Address": "text-format@r8r.test"}],
        "Text": "$regex:^Plain text body\\s*$",
        "HTML": ""
      }
      """

  Scenario: An HTML email is delivered
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "html-format@r8r.test",
        "subject": "r8r-bdd-send-email html-format",
        "emailFormat": "html",
        "html": "<p>Hello <b>world</b></p>",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email html-format"
    And the Mailpit message with subject "r8r-bdd-send-email html-format" matches:
      """
      {
        "HTML": "$contains:Hello <b>world</b>"
      }
      """

  Scenario: A "both" format email carries text and HTML parts
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "both-format@r8r.test",
        "subject": "r8r-bdd-send-email both-format",
        "emailFormat": "both",
        "text": "Both plain body",
        "html": "<p>Both HTML body</p>",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email both-format"
    And the Mailpit message with subject "r8r-bdd-send-email both-format" matches:
      """
      {"Text": "$contains:Both plain body"}
      """
    And the Mailpit message with subject "r8r-bdd-send-email both-format" matches:
      """
      {"HTML": "$contains:Both HTML body"}
      """

  Scenario: To, CC, BCC and Reply-To accept multiple addresses
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "to1@r8r.test, to2@r8r.test",
        "subject": "r8r-bdd-send-email recipient-lists",
        "emailFormat": "text",
        "text": "Body",
        "options": {
          "appendAttribution": false,
          "ccEmail": "cc1@r8r.test, cc2@r8r.test",
          "bccEmail": "bcc1@r8r.test",
          "replyTo": "reply@r8r.test"
        }
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email recipient-lists"
    And the Mailpit message with subject "r8r-bdd-send-email recipient-lists" matches:
      """
      {
        "To": [{"Address": "to1@r8r.test"}, {"Address": "to2@r8r.test"}],
        "Cc": [{"Address": "cc1@r8r.test"}, {"Address": "cc2@r8r.test"}],
        "Bcc": [{"Address": "bcc1@r8r.test"}],
        "ReplyTo": [{"Address": "reply@r8r.test"}]
      }
      """

  Scenario: An attachment from a binary property carries its name, mime type and content
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "report" with content "attachment file contents" and mime type "text/plain"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "attachment@r8r.test",
        "subject": "r8r-bdd-send-email single-attachment",
        "emailFormat": "text",
        "text": "See attached",
        "options": {"appendAttribution": false, "attachments": "report"}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email single-attachment"
    And the Mailpit message with subject "r8r-bdd-send-email single-attachment" has 1 attachments
    And the Mailpit message with subject "r8r-bdd-send-email single-attachment" matches:
      """
      {"Attachments": [{"FileName": "report.txt", "ContentType": "text/plain"}]}
      """
    And the Mailpit message with subject "r8r-bdd-send-email single-attachment" has an attachment "report.txt" with content "attachment file contents"

  Scenario: Multiple binary properties become multiple attachments
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "one" with content "first file" and mime type "text/plain"
    And the trigger item 0 has the binary property "two" with content "second file" and mime type "text/plain"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "multi-attachment@r8r.test",
        "subject": "r8r-bdd-send-email multi-attachment",
        "emailFormat": "text",
        "text": "See attached",
        "options": {"appendAttribution": false, "attachments": "one, two"}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email multi-attachment"
    And the Mailpit message with subject "r8r-bdd-send-email multi-attachment" has 2 attachments
    And the Mailpit message with subject "r8r-bdd-send-email multi-attachment" has an attachment "one.txt" with content "first file"
    And the Mailpit message with subject "r8r-bdd-send-email multi-attachment" has an attachment "two.txt" with content "second file"

  Scenario: Typeversion 2.1 appends the n8n attribution by default
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Send  | emailSend     | 2.1         |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "attribution-on@r8r.test",
        "subject": "r8r-bdd-send-email attribution-default-on",
        "emailFormat": "text",
        "text": "Body without attribution yet",
        "options": {}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email attribution-default-on"
    And the Mailpit message with subject "r8r-bdd-send-email attribution-default-on" matches:
      """
      {"Text": "$contains:This email was sent automatically with n8n"}
      """

  Scenario: Typeversion 2 does not append the n8n attribution by default
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Send  | emailSend     | 2           |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "attribution-v2@r8r.test",
        "subject": "r8r-bdd-send-email attribution-default-off-v2",
        "emailFormat": "text",
        "text": "Exact body v2",
        "options": {}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email attribution-default-off-v2"
    And the Mailpit message with subject "r8r-bdd-send-email attribution-default-off-v2" matches:
      """
      {"Text": "$regex:^Exact body v2\\s*$"}
      """

  Scenario: appendAttribution: false turns the attribution off explicitly
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Send  | emailSend     | 2.1         |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "attribution-off@r8r.test",
        "subject": "r8r-bdd-send-email attribution-explicit-off",
        "emailFormat": "text",
        "text": "Exact body attribution off",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email attribution-explicit-off"
    And the Mailpit message with subject "r8r-bdd-send-email attribution-explicit-off" matches:
      """
      {"Text": "$regex:^Exact body attribution off\\s*$"}
      """

  Scenario: Subject and body support expressions from the input item
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the trigger outputs the items:
      """
      [{"name": "Ada", "greeting": "Hello from expressions"}]
      """
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "expressions@r8r.test",
        "subject": "=r8r-bdd-send-email expressions {{ $json.name }}",
        "emailFormat": "text",
        "text": "={{ $json.greeting }}",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And Mailpit receives a message with subject "r8r-bdd-send-email expressions Ada"
    And the Mailpit message with subject "r8r-bdd-send-email expressions Ada" matches:
      """
      {"Text": "$contains:Hello from expressions"}
      """

  Scenario: The output item has nodemailer's result shape
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "output-shape@r8r.test",
        "subject": "r8r-bdd-send-email output-shape",
        "emailFormat": "text",
        "text": "Body",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs items matching:
      """
      [
        {
          "accepted": ["output-shape@r8r.test"],
          "rejected": [],
          "envelope": {"from": "sender@r8r.test", "to": ["output-shape@r8r.test"]},
          "messageId": "$nonempty",
          "response": "$nonempty"
        }
      ]
      """

  Scenario: An invalid recipient address fails with a clear error
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "not-an-email-address",
        "subject": "r8r-bdd-send-email invalid-recipient",
        "emailFormat": "text",
        "text": "Body",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the node "Send" failed with an error containing "toEmail"
    And Mailpit never receives a message with subject "r8r-bdd-send-email invalid-recipient"

  Scenario: A refused SMTP connection fails with a clear error and never leaks the password
    Given the credential "Bad SMTP" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 19999, "secure": false, "disableStartTls": true, "user": "nobody", "password": "s3cr3t-should-not-leak"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Bad SMTP"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "unreachable@r8r.test",
        "subject": "r8r-bdd-send-email connection-refused",
        "emailFormat": "text",
        "text": "Body",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the node "Send" failed with an error containing "SMTP"
    And the execution data does not contain "s3cr3t-should-not-leak"

  Scenario: A missing attachment binary property fails with a clear error
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | emailSend     |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "missing-attachment@r8r.test",
        "subject": "r8r-bdd-send-email missing-attachment",
        "emailFormat": "text",
        "text": "Body",
        "options": {"appendAttribution": false, "attachments": "doesNotExist"}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the node "Send" failed with an error containing "doesNotExist"
    And Mailpit never receives a message with subject "r8r-bdd-send-email missing-attachment"

  Scenario: continueOnFail turns a send failure into an error item
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          | onError               |
      | Start | manualTrigger |                        |
      | Send  | emailSend     | continueRegularOutput  |
      | After | noOp          |                        |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "fromEmail": "sender@r8r.test",
        "toEmail": "not-an-email-address",
        "subject": "r8r-bdd-send-email continue-on-fail",
        "emailFormat": "text",
        "text": "Body",
        "options": {"appendAttribution": false}
      }
      """
    And the connections "Start -> Send -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """
    And Mailpit never receives a message with subject "r8r-bdd-send-email continue-on-fail"

  Scenario: sendAndWait is not supported natively yet
    Given the credential "Mailpit" of type "smtp" with the data:
      """
      {"host": "127.0.0.1", "port": 1025, "secure": false, "disableStartTls": true, "user": "", "password": ""}
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Send  | emailSend     | 2.1         |
    And the node "Send" uses the "smtp" credential "Mailpit"
    And the node "Send" has parameters:
      """
      {
        "operation": "sendAndWait",
        "fromEmail": "sender@r8r.test",
        "toEmail": "wait@r8r.test",
        "subject": "r8r-bdd-send-email send-and-wait",
        "options": {}
      }
      """
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the node "Send" failed with an error containing "not supported natively yet"
