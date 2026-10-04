@spec-6.6 @phase-4 @node-email-imap @requires-imap
Feature: Email Trigger (IMAP)
  r8r's first long-lived trigger (plan task 1.15): faithful to n8n's
  `EmailReadImap` v2 node (`n8n-nodes-base.emailReadImap`, typeVersion
  2/2.1/2.2 -- the 2.35.7 editor creates 2.2). Connects to GreenMail
  (Docker `r8r-bdd-imap`, SMTP 127.0.0.1:3025, IMAP 127.0.0.1:3143, auth
  disabled -- any user/password works, and a mailbox is created on first
  delivery; opt in with R8R_BDD_INCLUDE=requires-imap), selects the
  configured mailbox, and polls it for mail matching the search criteria
  ("UNSEEN" by default), starting one execution per batch of new mail with
  that mail as the output items.

  Test mail is delivered straight into GreenMail over SMTP (not through an
  r8r node), so each scenario uses its own mailbox address to avoid
  cross-scenario interference.

  Because GreenMail's auth is disabled, "wrong credentials" is stood in
  for by an unreachable port: the node has no way to tell "the server
  rejected this login" from "the server could not be reached" until it
  tries to connect, and both surface the same way -- an activation error.

  Scenario: An unseen mail triggers one execution with simple format fields
    Given the credential "IMAP Simple" of type "imap" with the data:
      """
      {"user": "imap-simple@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP simple format" with nodes:
      | name | type          | parameters                                                                          |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Simple"
    And the workflow is active
    When I deliver a test email to "imap-simple@r8r.test" with subject "Hello r8r" and body "Hi there from GreenMail"
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"subject": "Hello r8r", "textPlain": "$contains:Hi there from GreenMail", "from": "$contains:bdd-sender", "to": "$contains:imap-simple"}]
      """

  Scenario: postProcessAction "read" marks the mail seen so it does not fire twice
    Given the credential "IMAP Read" of type "imap" with the data:
      """
      {"user": "imap-read@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP mark as read" with nodes:
      | name | type          | parameters |
      | Mail | emailReadImap |            |
    And the node "Mail" has parameters:
      """
      {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {"trackLastMessageId": false}}
      """
    And the node "Mail" uses the "imap" credential "IMAP Read"
    And the workflow is active
    When I deliver a test email to "imap-read@r8r.test" with subject "Read once" and body "Should fire once"
    Then within 10 seconds the workflow has at least 1 executions
    And I remember the executions count of the workflow
    Then after 6 seconds the workflow has no new executions

  Scenario: postProcessAction "nothing" relies on UID tracking to avoid firing twice
    Given the credential "IMAP Nothing" of type "imap" with the data:
      """
      {"user": "imap-nothing@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP nothing plus tracking" with nodes:
      | name | type          | parameters                                                                             |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "nothing", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Nothing"
    And the workflow is active
    When I deliver a test email to "imap-nothing@r8r.test" with subject "Still unseen" and body "Tracked by UID instead"
    Then within 10 seconds the workflow has at least 1 executions
    And I remember the executions count of the workflow
    Then after 6 seconds the workflow has no new executions

  Scenario: The "resolved" format returns the full parsed email
    Given the credential "IMAP Resolved" of type "imap" with the data:
      """
      {"user": "imap-resolved@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP resolved format" with nodes:
      | name | type          | parameters                                                                 |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "resolved"} |
    And the node "Mail" uses the "imap" credential "IMAP Resolved"
    And the workflow is active
    When I deliver a test email to "imap-resolved@r8r.test" with subject "Resolved format" and body "Plain body text"
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"subject": "Resolved format", "text": "$contains:Plain body text", "html": false}]
      """

  Scenario: The "raw" format returns the base64-encoded message
    Given the credential "IMAP Raw" of type "imap" with the data:
      """
      {"user": "imap-raw@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP raw format" with nodes:
      | name | type          | parameters                                                            |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "raw"} |
    And the node "Mail" uses the "imap" credential "IMAP Raw"
    And the workflow is active
    When I deliver a test email to "imap-raw@r8r.test" with subject "Raw format" and body "Raw body"
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"raw": "$regex:^[A-Za-z0-9+/=]+$"}]
      """

  Scenario: Attachments are downloaded to binary data with the configured prefix
    Given the credential "IMAP Attachments" of type "imap" with the data:
      """
      {"user": "imap-attach@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP attachments" with nodes:
      | name | type          | parameters |
      | Mail | emailReadImap |            |
    And the node "Mail" has parameters:
      """
      {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "downloadAttachments": true, "dataPropertyAttachmentsPrefixName": "myfile_", "options": {}}
      """
    And the node "Mail" uses the "imap" credential "IMAP Attachments"
    And the workflow is active
    When I deliver a test email to "imap-attach@r8r.test" with subject "Attachment test" and body "See attached" and an attachment "note.txt" with content "attachment-body"
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" output item 0 has the binary property "myfile_0" with file name "note.txt"

  Scenario: Custom search criteria restricts which mail triggers the workflow
    Given the credential "IMAP Criteria" of type "imap" with the data:
      """
      {"user": "imap-criteria@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP custom criteria" with nodes:
      | name | type          | parameters |
      | Mail | emailReadImap |            |
    And the node "Mail" has parameters:
      """
      {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {"customEmailConfig": "[\"UNSEEN\", [\"SUBJECT\", \"wanted-marker\"]]"}}
      """
    And the node "Mail" uses the "imap" credential "IMAP Criteria"
    And the workflow is active
    When I deliver a test email to "imap-criteria@r8r.test" with subject "ignore-this-one" and body "Not matching the custom rule"
    And I deliver a test email to "imap-criteria@r8r.test" with subject "has the wanted-marker inside" and body "Matches the custom rule"
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"subject": "has the wanted-marker inside"}]
      """
    And the node "Mail" outputs 1 items

  Scenario: Deactivating the workflow stops the listener
    Given the credential "IMAP Deactivate" of type "imap" with the data:
      """
      {"user": "imap-deactivate@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP stoppable" with nodes:
      | name | type          | parameters                                                                       |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Deactivate"
    And the workflow is active
    When I deactivate the workflow
    And I remember the executions count of the workflow
    And I deliver a test email to "imap-deactivate@r8r.test" with subject "Should not trigger" and body "Arrives after deactivation"
    Then after 6 seconds the workflow has no new executions

  Scenario: An unreachable IMAP server surfaces an activation error
    Given the credential "IMAP Unreachable" of type "imap" with the data:
      """
      {"user": "nouser@r8r.test", "password": "wrong-password", "host": "127.0.0.1", "port": 39999, "secure": false}
      """
    And a workflow named "IMAP bad connection" with nodes:
      | name | type          | parameters                                                                       |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Unreachable"
    When I activate the workflow
    Then the response status is a client error

  Scenario: The credential's password never appears in execution data
    Given the credential "IMAP Secret" of type "imap" with the data:
      """
      {"user": "imap-secret@r8r.test", "password": "sUperSecretPW123", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a workflow named "IMAP password hygiene" with nodes:
      | name | type          | parameters                                                                       |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Secret"
    And the workflow is active
    When I deliver a test email to "imap-secret@r8r.test" with subject "No leaks" and body "Nothing secret in here"
    Then within 10 seconds the workflow has at least 1 executions
    And the execution data does not contain "sUperSecretPW123"
