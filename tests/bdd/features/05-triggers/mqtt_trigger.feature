@spec-6.6 @phase-1 @node-mqtt @requires-mqtt
Feature: MQTT Trigger
  Faithful to n8n's `MqttTrigger.node.js` (typeVersion 1) +
  `GenericFunctions.js`'s `createClient`: subscribes to `topics`
  (comma-separated, each optionally suffixed `:qos`, default QoS 0; an
  out-of-range QoS falls back to 0) and emits `{message, topic}` per
  message.

  Background:
    Given a running r8r server with an owner and an API key
    And the credential "Test MQTT Trigger" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 1883, "clean": true}
      """

  Scenario: A published message triggers one execution with the default options
    Given a workflow named "MQTT default options" with nodes:
      | name | type       | parameters                                                      |
      | Mail | mqttTrigger | {"topics": "bdd/trigger/topic1", "options": {}} |
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic1":
      """
      hello from mqtt
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": "hello from mqtt", "topic": "bdd/trigger/topic1"}]
      """

  Scenario: jsonParseBody parses the message to an object
    Given a workflow named "MQTT json parse body" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "bdd/trigger/topic2", "options": {"jsonParseBody": true}}
      """
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic2":
      """
      {"order": 42}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": {"order": 42}, "topic": "bdd/trigger/topic2"}]
      """

  Scenario: onlyMessage returns just the message
    Given a workflow named "MQTT only message" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "bdd/trigger/topic3", "options": {"jsonParseBody": true, "onlyMessage": true}}
      """
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic3":
      """
      {"order": 99}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"order": 99}]
      """

  Scenario: A comma-separated topic list with a per-topic QoS subscribes to all of them
    Given a workflow named "MQTT multiple topics" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "bdd/trigger/topic4a:1,bdd/trigger/topic4b", "options": {}}
      """
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic4a":
      """
      from topic a
      """
    And I publish to the MQTT topic "bdd/trigger/topic4b":
      """
      from topic b
      """
    Then within 10 seconds the workflow has at least 2 executions

  Scenario: parallelProcessing false processes messages one at a time
    Given a workflow named "MQTT sequential processing" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "bdd/trigger/topic5", "options": {"parallelProcessing": false}}
      """
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic5":
      """
      sequential message
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": "sequential message", "topic": "bdd/trigger/topic5"}]
      """

  Scenario: Deactivating the workflow stops the listener
    Given a workflow named "MQTT stoppable" with nodes:
      | name | type        | parameters                                      |
      | Mail | mqttTrigger | {"topics": "bdd/trigger/topic6", "options": {}} |
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    And the workflow is active
    When I deactivate the workflow
    And I remember the executions count of the workflow
    And I publish to the MQTT topic "bdd/trigger/topic6":
      """
      should not trigger
      """
    Then after 6 seconds the workflow has no new executions

  Scenario: An unreachable MQTT broker surfaces an activation error
    Given the credential "Bad MQTT" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 59999, "clean": true}
      """
    And a workflow named "MQTT bad connection" with nodes:
      | name | type        | parameters                                      |
      | Mail | mqttTrigger | {"topics": "bdd/trigger/topic7", "options": {}} |
    And the node "Mail" uses the "mqtt" credential "Bad MQTT"
    When I activate the workflow
    Then the response status is a client error

  Scenario: Missing topics are rejected at activation
    Given a workflow named "MQTT no topics" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "", "options": {}}
      """
    And the node "Mail" uses the "mqtt" credential "Test MQTT Trigger"
    When I activate the workflow
    Then the response status is a client error

  Scenario: The credential's password never appears in execution data
    Given the credential "MQTT Secret" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 1883, "clean": true, "username": "r8r", "password": "sUperSecretPW123"}
      """
    And a workflow named "MQTT password hygiene" with nodes:
      | name | type        | parameters |
      | Mail | mqttTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topics": "bdd/trigger/topic8", "options": {}}
      """
    And the node "Mail" uses the "mqtt" credential "MQTT Secret"
    And the workflow is active
    When I publish to the MQTT topic "bdd/trigger/topic8":
      """
      no secrets here
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the execution data does not contain "sUperSecretPW123"
