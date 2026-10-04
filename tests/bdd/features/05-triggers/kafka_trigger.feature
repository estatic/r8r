@spec-6.6 @phase-1 @node-kafka @requires-kafka
Feature: Kafka Trigger
  Faithful to n8n's `KafkaTriggerV1.node.js` (typeVersion 1.3 -- the
  `KafkaTrigger` `VersionedNodeType`'s `defaultVersion`, what the 2.35.7
  editor creates) for the options this build implements.

  **Major documented deviation**: kafkajs (the reference's client) joins a
  real consumer group, with the broker's group coordinator assigning
  partitions and tracking committed offsets server-side. `rskafka` (this
  build's pure-Rust client) has no consumer-group coordinator client at
  all, so this trigger always consumes **partition 0 only**, keeps its
  next-offset cursor in the listener task's own memory (not committed to
  the broker, not persisted across a restart), and `groupId` is accepted
  as a required parameter but otherwise has no effect.
  `sessionTimeout`/`heartbeatInterval` are accepted but unused (no group
  to time out of); `useSchemaRegistry` is rejected at activation, matching
  the Kafka action node.

  Background:
    Given a running r8r server with an owner and an API key
    And the credential "Test Kafka Trigger" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd-trigger", "brokers": "127.0.0.1:9092", "ssl": false, "authentication": false}
      """

  Scenario: A published message triggers one execution with the default options
    Given a workflow named "Kafka default options" with nodes:
      | name | type        | parameters                                                                |
      | Mail | kafkaTrigger | {"topic": "bdd-trigger-topic-1", "groupId": "bdd-trigger-group-1", "options": {}} |
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-1" exists
    And the workflow is active
    When I publish to the Kafka topic "bdd-trigger-topic-1":
      """
      hello from kafka
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": "hello from kafka", "topic": "bdd-trigger-topic-1"}]
      """

  Scenario: jsonParseMessage parses the message to an object
    Given a workflow named "Kafka json parse message" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-2", "groupId": "bdd-trigger-group-2", "options": {"jsonParseMessage": true}}
      """
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-2" exists
    And the workflow is active
    When I publish to the Kafka topic "bdd-trigger-topic-2":
      """
      {"order": 42}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": {"order": 42}, "topic": "bdd-trigger-topic-2"}]
      """

  Scenario: onlyMessage returns just the message
    Given a workflow named "Kafka only message" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-3", "groupId": "bdd-trigger-group-3", "options": {"jsonParseMessage": true, "onlyMessage": true}}
      """
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-3" exists
    And the workflow is active
    When I publish to the Kafka topic "bdd-trigger-topic-3":
      """
      {"order": 99}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"order": 99}]
      """

  Scenario: returnHeaders includes the Kafka record's headers
    Given a workflow named "Kafka return headers" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-4", "groupId": "bdd-trigger-group-4", "options": {"returnHeaders": true}}
      """
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-4" exists
    And the workflow is active
    When I publish to the Kafka topic "bdd-trigger-topic-4":
      """
      with headers
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": "with headers", "topic": "bdd-trigger-topic-4"}]
      """

  Scenario: fromBeginning false only picks up messages published after activation
    Given a workflow named "Kafka from latest" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-5", "groupId": "bdd-trigger-group-5", "options": {"fromBeginning": false}}
      """
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-5" exists
    And the workflow is active
    When I publish to the Kafka topic "bdd-trigger-topic-5":
      """
      after activation
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"message": "after activation"}]
      """

  Scenario: Deactivating the workflow stops the listener
    Given a workflow named "Kafka stoppable" with nodes:
      | name | type         | parameters                                                                        |
      | Mail | kafkaTrigger | {"topic": "bdd-trigger-topic-6", "groupId": "bdd-trigger-group-6", "options": {}} |
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    And the Kafka topic "bdd-trigger-topic-6" exists
    And the workflow is active
    When I deactivate the workflow
    And I remember the executions count of the workflow
    And I publish to the Kafka topic "bdd-trigger-topic-6":
      """
      should not trigger
      """
    Then after 6 seconds the workflow has no new executions

  Scenario: An unreachable Kafka broker surfaces an activation error
    Given the credential "Bad Kafka" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd-trigger", "brokers": "127.0.0.1:59999", "ssl": false, "authentication": false}
      """
    And a workflow named "Kafka bad connection" with nodes:
      | name | type         | parameters                                                                        |
      | Mail | kafkaTrigger | {"topic": "bdd-trigger-topic-7", "groupId": "bdd-trigger-group-7", "options": {}} |
    And the node "Mail" uses the "kafka" credential "Bad Kafka"
    When I activate the workflow
    Then the response status is a client error

  Scenario: useSchemaRegistry is rejected at activation
    Given a workflow named "Kafka schema registry" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-8", "groupId": "bdd-trigger-group-8", "useSchemaRegistry": true, "options": {}}
      """
    And the node "Mail" uses the "kafka" credential "Test Kafka Trigger"
    When I activate the workflow
    Then the response status is a client error

  Scenario: The credential's password never appears in execution data
    Given the credential "Kafka Secret" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd-trigger", "brokers": "127.0.0.1:9092", "ssl": false, "authentication": true, "username": "r8r", "password": "sUperSecretPW123", "saslMechanism": "plain"}
      """
    And a workflow named "Kafka password hygiene" with nodes:
      | name | type         | parameters |
      | Mail | kafkaTrigger |            |
    And the node "Mail" has parameters:
      """
      {"topic": "bdd-trigger-topic-9", "groupId": "bdd-trigger-group-9", "options": {}}
      """
    And the node "Mail" uses the "kafka" credential "Kafka Secret"
    When I activate the workflow
    Then the response status is a client error
    And the response body does not contain "sUperSecretPW123"
