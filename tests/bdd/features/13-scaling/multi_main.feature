@spec-7.3 @phase-4
Feature: Multi-main leader election
  When two or more `r8r start` mains share a queue-mode database
  (`N8N_MULTI_MAIN_SETUP_ENABLED=true`, `EXECUTIONS_MODE=queue`), only the
  elected leader runs schedules, pollers and other long-lived triggers; a
  Redis lock (`SET NX EX`, renewed with a compare-and-expire script, plan
  task 4.1) decides who that is. Webhooks and forms stay registered on
  every main. Losing the lock (e.g. the leader process dies) hands
  leadership to the other main within the lock's TTL plus the check
  interval. Single-main, the default, is always leader and unaffected.

  Requires Redis (R8R_BDD_REDIS_HOST/PORT) and PostgreSQL
  (R8R_BDD_POSTGRES_URL); opt in with
  R8R_BDD_INCLUDE=requires-redis,requires-postgres.

  @requires-redis @requires-postgres
  Scenario: Two mains elect one leader and a schedule fires once per tick
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a running r8r worker named "w1"
    And a workflow named "Shared schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
      | Stamp    | set             |                                                                       |
    And the connections "Schedule -> Stamp"
    And the workflow is active
    Then after 5 seconds the workflow has between 3 and 6 executions

  @requires-redis @requires-postgres
  Scenario: Stopping the leader hands the schedule to the other main within the TTL
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a running r8r worker named "w1"
    And a workflow named "Takeover schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    And within 6 seconds the workflow has at least 2 executions
    When I stop the leader main
    And I remember the executions count of the workflow
    Then within 8 seconds the workflow has new executions

  @requires-redis @requires-postgres
  Scenario: A webhook workflow answers on every main, leader or not
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a running r8r worker named "w1"
    And a workflow named "Shared webhook" with nodes:
      | name    | type    | parameters                                                                          |
      | Webhook | webhook | {"httpMethod": "POST", "path": "shared", "responseMode": "lastNode", "options": {}} |
      | Echo    | set     |                                                                                      |
    And the node "Echo" sets the fields:
      """
      {"ok": true}
      """
    And the connections "Webhook -> Echo"
    And the workflow is active
    And the mains have synced
    When I send a POST request to "/webhook/shared" on the server named "main"
    Then the response status is 200
    When I send a POST request to "/webhook/shared" on the server named "b"
    Then the response status is 200

  @requires-redis @requires-postgres @requires-imap
  Scenario: Two mains share a mailbox and only the leader's listener processes it
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "2"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "1"
    And the credential "IMAP Shared" of type "imap" with the data:
      """
      {"user": "imap-multimain@r8r.test", "password": "testpass", "host": "127.0.0.1", "port": 3143, "secure": false}
      """
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a running r8r worker named "w1"
    And a workflow named "Shared mailbox" with nodes:
      | name | type          | parameters                                                                       |
      | Mail | emailReadImap | {"mailbox": "INBOX", "postProcessAction": "read", "format": "simple", "options": {}} |
    And the node "Mail" uses the "imap" credential "IMAP Shared"
    And the workflow is active
    When I deliver a test email to "imap-multimain@r8r.test" with subject "Multi-main mail" and body "Only one main should pick this up"
    Then within 10 seconds the workflow has at least 1 executions
    And the workflow has 1 execution

  @requires-redis @requires-postgres @requires-rabbitmq @requires-mqtt @requires-kafka
  Scenario: Two mains share RabbitMQ, Kafka and MQTT triggers and only the leader's listener processes each
    # A longer TTL/check interval than the other scenarios here: activating
    # three broker connections one after another is slower than a schedule
    # or webhook, and the Kafka Trigger has no committed offset to resume
    # from (documented deviation in kafka.rs -- rskafka has no
    # consumer-group support) -- its in-memory cursor resets to "latest"
    # every time its listener (re)starts, so a leadership flip between the
    # publish and the leader processing it would miss the message
    # entirely rather than just duplicate it. `fromBeginning: false` plus a
    # pre-created topic (not a historical backlog from earlier test runs
    # against this long-lived broker) keeps the scenario repeatable.
    Given queue mode backed by Redis and PostgreSQL
    And the environment variable "N8N_MULTI_MAIN_SETUP_ENABLED" is "true"
    And the environment variable "N8N_MULTI_MAIN_SETUP_KEY_TTL" is "10"
    And the environment variable "N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL" is "2"
    And the RabbitMQ queue "bdd-multimain-queue" does not exist
    And the Kafka topic "bdd-multimain-topic-2" exists
    And the credential "RabbitMQ Shared" of type "rabbitmq" with the data:
      """
      {"hostname": "127.0.0.1", "port": 5672, "username": "guest", "password": "guest", "vhost": "/"}
      """
    And the credential "Kafka Shared" of type "kafka" with the data:
      """
      {"clientId": "r8r-bdd-multimain", "brokers": "127.0.0.1:9092", "ssl": false, "authentication": false}
      """
    And the credential "MQTT Shared" of type "mqtt" with the data:
      """
      {"protocol": "mqtt", "host": "127.0.0.1", "port": 1883, "clean": true}
      """
    And a running r8r server with an owner and an API key
    And a running r8r server named "b" with the same configuration
    And a running r8r worker named "w1"
    And a workflow named "Shared RabbitMQ" with nodes:
      | name | type            | parameters                                               |
      | Mail | rabbitmqTrigger | {"queue": "bdd-multimain-queue", "options": {}}          |
    And the node "Mail" uses the "rabbitmq" credential "RabbitMQ Shared"
    And the workflow is active
    And a workflow named "Shared Kafka" with nodes:
      | name | type         | parameters                                                                             |
      | Mail | kafkaTrigger | {"topic": "bdd-multimain-topic-2", "groupId": "bdd-multimain-group", "options": {"fromBeginning": false}} |
    And the node "Mail" uses the "kafka" credential "Kafka Shared"
    And the workflow is active
    And a workflow named "Shared MQTT" with nodes:
      | name | type        | parameters                                         |
      | Mail | mqttTrigger | {"topics": "bdd/multimain/topic", "options": {}}   |
    And the node "Mail" uses the "mqtt" credential "MQTT Shared"
    And the workflow is active
    When I publish to the RabbitMQ queue "bdd-multimain-queue":
      """
      one leader only
      """
    And I publish to the Kafka topic "bdd-multimain-topic-2":
      """
      one leader only
      """
    And I publish to the MQTT topic "bdd/multimain/topic":
      """
      one leader only
      """
    Then within 10 seconds the workflow "Shared RabbitMQ" has at least 1 executions
    And within 10 seconds the workflow "Shared Kafka" has at least 1 executions
    And within 10 seconds the workflow "Shared MQTT" has at least 1 executions
    And the workflow "Shared RabbitMQ" has 1 execution
    And the workflow "Shared Kafka" has 1 execution
    And the workflow "Shared MQTT" has 1 execution

  Scenario: Single-main (the default) still fires schedules
    Given a running r8r server with an owner and an API key
    And a workflow named "Solo schedule" with nodes:
      | name     | type            | parameters                                                           |
      | Schedule | scheduleTrigger | {"rule": {"interval": [{"field": "seconds", "secondsInterval": 1}]}} |
    And the workflow is active
    Then within 6 seconds the workflow has at least 3 executions
