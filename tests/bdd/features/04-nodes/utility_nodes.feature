@spec-6.6 @phase-1 @node-utility
Feature: Utility nodes
  No Operation, Stop and Error, Crypto, Date & Time, Compare Datasets and
  friends from the native GA list.

  Scenario: No Operation passes items through
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Noop  | noOp          |
    And the connections "Start -> Noop"
    And the trigger outputs the items:
      """
      [{"a": 1}]
      """
    When I execute the workflow
    Then the node "Noop" outputs:
      """
      [{"a": 1}]
      """

  Scenario: Stop and Error with an error object
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Stop  | stopAndError  |
    And the node "Stop" has parameters:
      """
      {"errorType": "errorObject", "errorObject": "{\"code\": \"E_LIMIT\", \"message\": \"Over quota\"}"}
      """
    And the connections "Start -> Stop"
    When I execute the workflow
    Then the execution fails
    And the node "Stop" failed with an error containing "Over quota"

  Scenario Outline: Crypto hashes values
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Hash  | crypto        |
    And the node "Hash" has parameters:
      """
      {"action": "hash", "type": "<algorithm>", "value": "={{ $json.text }}", "dataPropertyName": "digest", "encoding": "hex"}
      """
    And the connections "Start -> Hash"
    And the trigger outputs the items:
      """
      [{"text": "abc"}]
      """
    When I execute the workflow
    Then the field "digest" of item 0 from the node "Hash" is "<digest>"

    Examples:
      | algorithm | digest                                                           |
      | MD5       | 900150983cd24fb0d6963f7d28e17f72                                 |
      | SHA1      | a9993e364706816aba3e25717850c26c9cd0d89d                         |
      | SHA256    | ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad |

  Scenario: Crypto computes an HMAC
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Sign  | crypto        |
    And the node "Sign" has parameters:
      """
      {"action": "hmac", "type": "SHA256", "value": "payload", "secret": "key", "dataPropertyName": "signature", "encoding": "hex"}
      """
    And the connections "Start -> Sign"
    When I execute the workflow
    Then the field "signature" of item 0 from the node "Sign" is "5d98b45c90a207fa998ce639fea6f02ecc8cc3f36fef81d694fb856b4d0a28ca"

  Scenario: Date & Time formats a date
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Format | dateTime      |
    And the node "Format" has parameters:
      """
      {"operation": "formatDate", "date": "={{ $json.when }}", "format": "custom", "customFormat": "yyyy/MM/dd HH:mm", "outputFieldName": "formatted", "options": {}}
      """
    And the connections "Start -> Format"
    And the trigger outputs the items:
      """
      [{"when": "2024-07-04T15:30:00Z"}]
      """
    When I execute the workflow
    Then the field "formatted" of item 0 from the node "Format" is "2024/07/04 15:30"

  Scenario: Date & Time adds to a date
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Add   | dateTime      |
    And the node "Add" has parameters:
      """
      {"operation": "addToDate", "magnitude": "={{ $json.when }}", "timeUnit": "days", "duration": 3, "outputFieldName": "later", "options": {}}
      """
    And the connections "Start -> Add"
    And the trigger outputs the items:
      """
      [{"when": "2024-02-27T00:00:00Z"}]
      """
    When I execute the workflow
    Then the field "later" of item 0 from the node "Add" is "$regex:^2024-03-01T00:00:00"

  Scenario: Markdown converts to HTML
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Md    | markdown      |
    And the node "Md" has parameters:
      """
      {"mode": "markdownToHtml", "markdown": "# Title", "destinationKey": "html", "options": {}}
      """
    And the connections "Start -> Md"
    When I execute the workflow
    Then the field "html" of item 0 from the node "Md" is "$contains:<h1"

  Scenario: XML converts to JSON
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Xml   | xml           |
    And the node "Xml" has parameters:
      """
      {"mode": "xmlToJson", "dataPropertyName": "xml", "options": {"explicitArray": false}}
      """
    And the connections "Start -> Xml"
    And the trigger outputs the items:
      """
      [{"xml": "<order><id>7</id></order>"}]
      """
    When I execute the workflow
    Then the node "Xml" outputs:
      """
      [{"order": {"id": "7"}}]
      """

  Scenario: Compare Datasets splits items into four outputs
    Given a workflow with nodes:
      | name    | type            | position |
      | Start   | manualTrigger   | 0,0      |
      | A       | code            | 200,-100 |
      | B       | code            | 200,100  |
      | Compare | compareDatasets | 400,0    |
    And the node "A" runs the JavaScript:
      """
      return [{ json: { id: 1, v: 'same' } }, { json: { id: 2, v: 'old' } }, { json: { id: 3, v: 'only-a' } }];
      """
    And the node "B" runs the JavaScript:
      """
      return [{ json: { id: 1, v: 'same' } }, { json: { id: 2, v: 'new' } }, { json: { id: 4, v: 'only-b' } }];
      """
    And the node "Compare" has parameters:
      """
      {"mergeByFields": {"values": [{"field1": "id", "field2": "id"}]}, "options": {}}
      """
    And the connections:
      """
      Start -> A
      Start -> B
      A -> Compare:0
      B -> Compare:1
      """
    When I execute the workflow
    Then output 0 of the node "Compare" has items matching:
      """
      [{"id": 3}]
      """
    And output 1 of the node "Compare" has 1 item
    And output 2 of the node "Compare" has 1 item
    And output 3 of the node "Compare" has items matching:
      """
      [{"id": 4}]
      """
