@spec-6.6 @phase-4 @node-mqtt @requires-mqtt
Feature: MQTT node
  `publish` against the `mqtt` credential. Faithful to n8n's `Mqtt.node.js`
  (typeVersion 1): the node passes its input items straight through
  (`return [items]`), it does not mark them `{success: true}`.

  Background:
    Given the credential "Test MQTT" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 1883, "clean": true}
      """

  Scenario: Send input data as JSON to a topic
    Given I am subscribed to the MQTT topic "bdd/mqtt/topic1"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | mqtt          |
    And the node "Send" has parameters:
      """
      {"topic": "bdd/mqtt/topic1", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "mqtt" credential "Test MQTT"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the MQTT topic "bdd/mqtt/topic1" receives a message matching:
      """
      {}
      """

  Scenario: Send a literal message with QoS 1 and retain
    Given I am subscribed to the MQTT topic "bdd/mqtt/topic2"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | mqtt          |
    And the node "Send" has parameters:
      """
      {"topic": "bdd/mqtt/topic2", "sendInputData": false, "message": "hello mqtt", "options": {"qos": 1, "retain": true}}
      """
    And the node "Send" uses the "mqtt" credential "Test MQTT"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the MQTT topic "bdd/mqtt/topic2" receives a message matching:
      """
      "hello mqtt"
      """

  Scenario: The node passes input items through unchanged
    Given I am subscribed to the MQTT topic "bdd/mqtt/topic3"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | set           |
      | Send  | mqtt          |
    And the node "Set" has parameters:
      """
      {"mode": "manual", "assignments": {"assignments": [{"id": "1", "name": "greeting", "value": "hi", "type": "string"}]}}
      """
    And the node "Send" has parameters:
      """
      {"topic": "bdd/mqtt/topic3", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "mqtt" credential "Test MQTT"
    And the connections "Start -> Set -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs:
      """
      [{"greeting": "hi"}]
      """

  Scenario: Unreachable broker fails the node, and its password never leaks into execution data
    Given the credential "Bad MQTT" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 18830, "username": "bdd", "password": "sUp3rS3cr3tMqttPassphrase!"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | mqtt          |
    And the node "Send" has parameters:
      """
      {"topic": "bdd/mqtt/unreachable", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "mqtt" credential "Bad MQTT"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sUp3rS3cr3tMqttPassphrase!"

  Scenario: onError "continueRegularOutput" turns a connection failure into a single error item
    Given the credential "Bad MQTT 2" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 18831}
      """
    And a workflow with nodes:
      | name  | type          | onError               |
      | Start | manualTrigger |                        |
      | Send  | mqtt          | continueRegularOutput  |
      | After | noOp          |                        |
    And the node "Send" has parameters:
      """
      {"topic": "bdd/mqtt/unreachable2", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "mqtt" credential "Bad MQTT 2"
    And the connections "Start -> Send -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """
