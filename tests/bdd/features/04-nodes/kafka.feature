@spec-6.6 @phase-4 @node-kafka @requires-kafka
Feature: Kafka node
  `send` against the `kafka` credential. Faithful to n8n's `KafkaV1.node.js`
  (typeVersion 1, the `VersionedNodeType`'s `defaultVersion` -- what the
  editor creates for a brand-new node).

  Background:
    Given the credential "Test Kafka" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd", "brokers": "127.0.0.1:9092", "ssl": false, "authentication": false}
      """

  Scenario: Send input data as JSON to a topic
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | kafka         |
    And the node "Send" has parameters:
      """
      {"topic": "bdd-kafka-topic-1", "sendInputData": true, "jsonParameters": false, "useSchemaRegistry": false, "useKey": false, "options": {}}
      """
    And the node "Send" uses the "kafka" credential "Test Kafka"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs items matching:
      """
      [{"topicName": "bdd-kafka-topic-1", "partition": 0, "errorCode": 0}]
      """
    And the Kafka topic "bdd-kafka-topic-1" receives a message matching:
      """
      {"value": {}}
      """

  Scenario: Send a literal message with a key and headers
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | kafka         |
    And the node "Send" has parameters:
      """
      {
        "topic": "bdd-kafka-topic-2",
        "sendInputData": false,
        "message": "hello kafka",
        "jsonParameters": false,
        "useSchemaRegistry": false,
        "useKey": true,
        "key": "order-42",
        "headersUi": {"headerValues": [{"key": "x-test", "value": "bdd"}]},
        "options": {}
      }
      """
    And the node "Send" uses the "kafka" credential "Test Kafka"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the Kafka topic "bdd-kafka-topic-2" receives a message matching:
      """
      {"value": "hello kafka", "key": "order-42", "headers": {"x-test": "bdd"}}
      """

  Scenario: Headers as JSON
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | kafka         |
    And the node "Send" has parameters:
      """
      {
        "topic": "bdd-kafka-topic-3",
        "sendInputData": false,
        "message": "with json headers",
        "jsonParameters": true,
        "useSchemaRegistry": false,
        "useKey": false,
        "headerParametersJson": "{\"x-json\": \"yes\"}",
        "options": {"compression": true}
      }
      """
    And the node "Send" uses the "kafka" credential "Test Kafka"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the Kafka topic "bdd-kafka-topic-3" receives a message matching:
      """
      {"value": "with json headers", "headers": {"x-json": "yes"}}
      """

  Scenario: Schema registry is not supported natively yet
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | kafka         |
    And the node "Send" has parameters:
      """
      {"topic": "bdd-kafka-topic-4", "sendInputData": true, "useSchemaRegistry": true, "schemaRegistryUrl": "http://127.0.0.1:9999", "eventName": "ns.event"}
      """
    And the node "Send" uses the "kafka" credential "Test Kafka"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported"

  Scenario: Unreachable broker fails the node, and its password never leaks into execution data
    Given the credential "Bad Kafka" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd", "brokers": "127.0.0.1:19092", "ssl": false, "authentication": true, "username": "bdd", "password": "sUp3rS3cr3tKafkaPassphrase!", "saslMechanism": "plain"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | kafka         |
    And the node "Send" has parameters:
      """
      {"topic": "bdd-kafka-unreachable", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "kafka" credential "Bad Kafka"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sUp3rS3cr3tKafkaPassphrase!"

  Scenario: onError "continueRegularOutput" turns a connection failure into a single error item
    Given the credential "Bad Kafka 2" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd", "brokers": "127.0.0.1:19093", "ssl": false}
      """
    And a workflow with nodes:
      | name  | type          | onError               |
      | Start | manualTrigger |                        |
      | Send  | kafka         | continueRegularOutput |
      | After | noOp          |                        |
    And the node "Send" has parameters:
      """
      {"topic": "bdd-kafka-unreachable2", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "kafka" credential "Bad Kafka 2"
    And the connections "Start -> Send -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """
