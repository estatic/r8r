@spec-6.6 @phase-4 @node-rabbitmq @requires-rabbitmq
Feature: RabbitMQ node
  `sendMessage` (mode queue | exchange) against the `rabbitmq` credential.
  Faithful to n8n's `RabbitMQ.node.js` (typeVersion 1.2).

  Background:
    Given the credential "Test RabbitMQ" of type "rabbitmq" with the data:
      """
      {"hostname": "127.0.0.1", "port": 5672, "username": "guest", "password": "guest", "vhost": "/"}
      """

  Scenario: Send input data to a queue
    Given the RabbitMQ queue "bdd-rabbitmq-queue-1" is empty
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {"operation": "sendMessage", "mode": "queue", "queue": "bdd-rabbitmq-queue-1", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs:
      """
      [{"success": true}]
      """
    And the RabbitMQ queue "bdd-rabbitmq-queue-1" receives a message matching:
      """
      {"body": {}}
      """

  Scenario: Send a literal message with custom headers to a queue
    Given the RabbitMQ queue "bdd-rabbitmq-queue-2" is empty
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {
        "operation": "sendMessage",
        "mode": "queue",
        "queue": "bdd-rabbitmq-queue-2",
        "sendInputData": false,
        "message": "hello from r8r",
        "options": {
          "headers": {"header": [{"key": "x-test", "value": "bdd"}]}
        }
      }
      """
    And the node "Send" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the RabbitMQ queue "bdd-rabbitmq-queue-2" receives a message matching:
      """
      {"body": "hello from r8r", "headers": {"x-test": "bdd"}}
      """

  Scenario: Send to a fanout exchange with a routing key
    Given the RabbitMQ queue "bdd-rabbitmq-queue-3" is empty
    And the RabbitMQ fanout exchange "bdd-rabbitmq-exchange-1" routes to the queue "bdd-rabbitmq-queue-3"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {
        "operation": "sendMessage",
        "mode": "exchange",
        "exchange": "bdd-rabbitmq-exchange-1",
        "exchangeType": "fanout",
        "routingKey": "",
        "sendInputData": true,
        "options": {}
      }
      """
    And the node "Send" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the node "Send" outputs:
      """
      [{"success": true}]
      """
    And the RabbitMQ queue "bdd-rabbitmq-queue-3" receives a message matching:
      """
      {"body": {}}
      """

  Scenario: Sending to a queue that does not exist fails, like n8n's checkQueue
    Given the RabbitMQ queue "bdd-rabbitmq-missing" does not exist
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {"operation": "sendMessage", "mode": "queue", "queue": "bdd-rabbitmq-missing", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the node "Send" failed with an error containing "NOT_FOUND"

  Scenario: With assertQueue the node declares a missing queue with its options
    Given the RabbitMQ queue "bdd-rabbitmq-asserted" does not exist
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {"operation": "sendMessage", "mode": "queue", "queue": "bdd-rabbitmq-asserted", "sendInputData": true, "options": {"assertQueue": true, "durable": false, "autoDelete": false}}
      """
    And the node "Send" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution succeeds
    And the RabbitMQ queue "bdd-rabbitmq-asserted" receives a message matching:
      """
      {"body": {}}
      """

  Scenario: Delete From Queue is not supported outside a trigger
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | rabbitmq      |
    And the node "Delete" has parameters:
      """
      {"operation": "deleteMessage"}
      """
    And the node "Delete" uses the "rabbitmq" credential "Test RabbitMQ"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported"

  Scenario: Unreachable broker fails the node, and its password never leaks into execution data
    Given the credential "Bad RabbitMQ" of type "rabbitmq" with the data:
      """
      {"hostname": "127.0.0.1", "port": 56720, "username": "guest", "password": "sUp3rS3cr3tRabbitPassphrase!", "vhost": "/"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Send  | rabbitmq      |
    And the node "Send" has parameters:
      """
      {"operation": "sendMessage", "mode": "queue", "queue": "bdd-rabbitmq-unreachable", "sendInputData": true, "options": {}}
      """
    And the node "Send" uses the "rabbitmq" credential "Bad RabbitMQ"
    And the connections "Start -> Send"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sUp3rS3cr3tRabbitPassphrase!"
