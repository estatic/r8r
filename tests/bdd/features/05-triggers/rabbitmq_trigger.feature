@spec-6.6 @phase-1 @node-rabbitmq @requires-rabbitmq
Feature: RabbitMQ Trigger
  Faithful to n8n's `RabbitMQTrigger.node.js` (typeVersion 1) +
  `GenericFunctions.js`'s `handleMessage`/`rabbitmqConnectQueue`. Unlike the
  RabbitMQ action node (always a passive `checkQueue`), the Trigger's own
  `options` collection spreads in `rabbitDefaultOptions`, whose
  `assertQueue` defaults to `true`: activation declares the queue
  (durable) rather than merely checking it exists.

  "Specified Later in Workflow" acknowledge mode is not supported natively
  (it needs a later RabbitMQ node's `deleteMessage` to ack/nack this
  trigger's delivery, which r8r's executor has no path for) and is
  rejected at activation with a clear error.

  Background:
    Given a running r8r server with an owner and an API key
    And the credential "Test RabbitMQ Trigger" of type "rabbitmq" with the data:
      """
      {"hostname": "127.0.0.1", "port": 5672, "username": "guest", "password": "guest", "vhost": "/"}
      """

  Scenario: A published message triggers one execution with the default options
    Given the RabbitMQ queue "bdd-trigger-queue-1" does not exist
    And a workflow named "RabbitMQ default options" with nodes:
      | name  | type            | parameters                                        |
      | Mail  | rabbitmqTrigger | {"queue": "bdd-trigger-queue-1", "options": {}}   |
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-1":
      """
      hello from rabbitmq
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"content": "hello from rabbitmq"}]
      """

  Scenario: jsonParseBody parses the content to an object
    Given the RabbitMQ queue "bdd-trigger-queue-2" does not exist
    And a workflow named "RabbitMQ json parse body" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-2", "options": {"jsonParseBody": true}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-2":
      """
      {"order": 42}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"content": {"order": 42}}]
      """

  Scenario: onlyContent returns just the content
    Given the RabbitMQ queue "bdd-trigger-queue-3" does not exist
    And a workflow named "RabbitMQ only content" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-3", "options": {"jsonParseBody": true, "onlyContent": true}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-3":
      """
      {"order": 99}
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" outputs items matching:
      """
      [{"order": 99}]
      """

  Scenario: contentIsBinary saves the content as binary data
    Given the RabbitMQ queue "bdd-trigger-queue-4" does not exist
    And a workflow named "RabbitMQ content is binary" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-4", "options": {"contentIsBinary": true}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-4":
      """
      binary payload
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the node "Mail" output item 0 has the binary property "data" with file name ""

  Scenario: acknowledge "immediately" acks before the execution finishes
    Given the RabbitMQ queue "bdd-trigger-queue-5" does not exist
    And a workflow named "RabbitMQ ack immediately" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-5", "options": {"acknowledge": "immediately"}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-5":
      """
      ack immediately
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the RabbitMQ queue "bdd-trigger-queue-5" is empty

  Scenario: A failed execution with executionFinishesSuccessfully leaves the message requeued
    Given the RabbitMQ queue "bdd-trigger-queue-6" does not exist
    And a workflow named "RabbitMQ ack on success only" with nodes:
      | name  | type            | parameters                                                                                      |
      | Mail  | rabbitmqTrigger | {"queue": "bdd-trigger-queue-6", "options": {"acknowledge": "executionFinishesSuccessfully"}}  |
      | Throw | code            | {"mode": "runOnceForEachItem", "language": "javaScript", "jsCode": "throw new Error('boom')"}  |
    And the connections "Mail -> Throw"
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-6":
      """
      will fail and requeue
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the RabbitMQ queue "bdd-trigger-queue-6" receives a message matching:
      """
      {"body": "will fail and requeue"}
      """

  Scenario: Deactivating the workflow stops the listener
    Given the RabbitMQ queue "bdd-trigger-queue-7" does not exist
    And a workflow named "RabbitMQ stoppable" with nodes:
      | name | type            | parameters                                      |
      | Mail | rabbitmqTrigger | {"queue": "bdd-trigger-queue-7", "options": {}} |
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I deactivate the workflow
    And I remember the executions count of the workflow
    And I publish to the RabbitMQ queue "bdd-trigger-queue-7":
      """
      should not trigger
      """
    Then after 6 seconds the workflow has no new executions

  Scenario: An unreachable RabbitMQ server surfaces an activation error
    Given the credential "Bad RabbitMQ" of type "rabbitmq" with the data:
      """
      {"hostname": "127.0.0.1", "port": 59999, "username": "guest", "password": "guest", "vhost": "/"}
      """
    And a workflow named "RabbitMQ bad connection" with nodes:
      | name | type            | parameters                                      |
      | Mail | rabbitmqTrigger | {"queue": "bdd-trigger-queue-8", "options": {}} |
    And the node "Mail" uses the "rabbitmq" credential "Bad RabbitMQ"
    When I activate the workflow
    Then the response status is a client error

  Scenario: acknowledge "laterMessageNode" is rejected at activation
    Given a workflow named "RabbitMQ later message node" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-9", "options": {"acknowledge": "laterMessageNode"}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    When I activate the workflow
    Then the response status is a client error

  Scenario: The credential's password never appears in execution data
    Given the RabbitMQ queue "bdd-trigger-queue-10" does not exist
    And a workflow named "RabbitMQ password hygiene" with nodes:
      | name | type            | parameters |
      | Mail | rabbitmqTrigger |            |
    And the node "Mail" has parameters:
      """
      {"queue": "bdd-trigger-queue-10", "options": {}}
      """
    And the node "Mail" uses the "rabbitmq" credential "Test RabbitMQ Trigger"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-trigger-queue-10":
      """
      no secrets here
      """
    Then within 10 seconds the workflow has at least 1 executions
    And the execution data does not contain "guest"
