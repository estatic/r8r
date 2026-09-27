@spec-6.6 @phase-4 @node-redis @requires-redis
Feature: Redis node
  Get, send and update data in Redis against the `redis` credential.
  Faithful to n8n's `Redis.node.js` (typeVersion 1): `delete`, `get`, `incr`,
  `info`, `keys`, `pop`, `publish`, `push`, `set`.

  Scenario: Set and get a string value
    Given redis has no keys matching "bdd:redis:string:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379, "database": 0}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | redis         |
      | Get   | redis         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:string:1", "value": "hello world", "keyType": "string", "expire": false}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:string:1", "keyType": "string", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"value": "hello world"}]
      """
    And redis has no keys matching "bdd:redis:string:*"

  Scenario: Set with a JSON value and get a hash, in insertion order
    Given redis has no keys matching "bdd:redis:hash:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | redis         |
      | Get   | redis         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:hash:1", "value": "{\"name\": \"Ada\", \"role\": \"engineer\"}", "keyType": "hash", "valueIsJSON": true}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:hash:1", "keyType": "hash", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"value": {"name": "Ada", "role": "engineer"}}]
      """
    And redis has no keys matching "bdd:redis:hash:*"

  Scenario: Push (tail) builds a list in append order, read back with get
    Given redis has no keys matching "bdd:redis:list:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | PushA  | redis         |
      | PushB  | redis         |
      | PushC  | redis         |
      | Get    | redis         |
    And the node "PushA" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:list:1", "messageData": "a", "tail": true}
      """
    And the node "PushA" uses the "redis" credential "Test Redis"
    And the node "PushB" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:list:1", "messageData": "b", "tail": true}
      """
    And the node "PushB" uses the "redis" credential "Test Redis"
    And the node "PushC" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:list:1", "messageData": "c", "tail": true}
      """
    And the node "PushC" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:list:1", "keyType": "list", "propertyName": "items", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> PushA -> PushB -> PushC -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"items": ["a", "b", "c"]}]
      """
    And redis has no keys matching "bdd:redis:list:*"

  Scenario: Pop from the head and the tail, with JSON auto-parsing
    Given redis has no keys matching "bdd:redis:pop:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | PushFirst | redis         |
      | PushJson  | redis         |
      | PushLast  | redis         |
      | PopHead   | redis         |
      | PopJson   | redis         |
      | PopTail   | redis         |
    And the node "PushFirst" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:pop:1", "messageData": "first", "tail": true}
      """
    And the node "PushFirst" uses the "redis" credential "Test Redis"
    And the node "PushJson" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:pop:1", "messageData": "{\"x\": 1}", "tail": true}
      """
    And the node "PushJson" uses the "redis" credential "Test Redis"
    And the node "PushLast" has parameters:
      """
      {"operation": "push", "list": "bdd:redis:pop:1", "messageData": "last", "tail": true}
      """
    And the node "PushLast" uses the "redis" credential "Test Redis"
    And the node "PopHead" has parameters:
      """
      {"operation": "pop", "list": "bdd:redis:pop:1", "tail": false, "propertyName": "value", "options": {}}
      """
    And the node "PopHead" uses the "redis" credential "Test Redis"
    And the node "PopJson" has parameters:
      """
      {"operation": "pop", "list": "bdd:redis:pop:1", "tail": false, "propertyName": "value", "options": {}}
      """
    And the node "PopJson" uses the "redis" credential "Test Redis"
    And the node "PopTail" has parameters:
      """
      {"operation": "pop", "list": "bdd:redis:pop:1", "tail": true, "propertyName": "value", "options": {}}
      """
    And the node "PopTail" uses the "redis" credential "Test Redis"
    And the connections "Start -> PushFirst -> PushJson -> PushLast -> PopHead -> PopJson -> PopTail"
    When I execute the workflow
    Then the execution succeeds
    And the node "PopHead" outputs:
      """
      [{"value": "first"}]
      """
    And the node "PopJson" outputs:
      """
      [{"value": {"x": 1}}]
      """
    And the node "PopTail" outputs:
      """
      [{"value": "last"}]
      """
    And redis has no keys matching "bdd:redis:pop:*"

  Scenario: Set membership via keyType "sets"
    Given redis has no keys matching "bdd:redis:set:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | redis         |
      | Get   | redis         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:set:1", "value": "={{ ['red', 'green'] }}", "keyType": "sets"}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:set:1", "keyType": "sets", "propertyName": "members", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"members": ["red", "green"]}]
      """
    And redis has no keys matching "bdd:redis:set:*"

  Scenario: Increment atomically and set an expiry
    Given redis has no keys matching "bdd:redis:incr:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | IncrA  | redis         |
      | IncrB  | redis         |
    And the node "IncrA" has parameters:
      """
      {"operation": "incr", "key": "bdd:redis:incr:1", "expire": true, "ttl": 30}
      """
    And the node "IncrA" uses the "redis" credential "Test Redis"
    And the node "IncrB" has parameters:
      """
      {"operation": "incr", "key": "bdd:redis:incr:1", "expire": false}
      """
    And the node "IncrB" uses the "redis" credential "Test Redis"
    And the connections "Start -> IncrA -> IncrB"
    When I execute the workflow
    Then the execution succeeds
    And the node "IncrA" outputs:
      """
      [{"bdd:redis:incr:1": 1}]
      """
    And the node "IncrB" outputs:
      """
      [{"bdd:redis:incr:1": 2}]
      """
    And the redis key "bdd:redis:incr:1" has a ttl greater than 0 seconds
    And redis has no keys matching "bdd:redis:incr:*"

  Scenario: Keys with a pattern and getValues returns matching key/value pairs
    Given redis has no keys matching "bdd:redis:keys:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | SetA    | redis         |
      | SetB    | redis         |
      | ListAll | redis         |
    And the node "SetA" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:keys:a", "value": "1", "keyType": "string"}
      """
    And the node "SetA" uses the "redis" credential "Test Redis"
    And the node "SetB" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:keys:b", "value": "2", "keyType": "string"}
      """
    And the node "SetB" uses the "redis" credential "Test Redis"
    And the node "ListAll" has parameters:
      """
      {"operation": "keys", "keyPattern": "bdd:redis:keys:*", "getValues": true}
      """
    And the node "ListAll" uses the "redis" credential "Test Redis"
    And the connections "Start -> SetA -> SetB -> ListAll"
    When I execute the workflow
    Then the execution succeeds
    And the node "ListAll" outputs:
      """
      [{"bdd:redis:keys:a": "1", "bdd:redis:keys:b": "2"}]
      """
    And redis has no keys matching "bdd:redis:keys:*"

  Scenario: Keys without getValues returns only the key names
    Given redis has no keys matching "bdd:redis:onlykey:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | SetA    | redis         |
      | ListAll | redis         |
    And the node "SetA" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:onlykey:a", "value": "1", "keyType": "string"}
      """
    And the node "SetA" uses the "redis" credential "Test Redis"
    And the node "ListAll" has parameters:
      """
      {"operation": "keys", "keyPattern": "bdd:redis:onlykey:*", "getValues": false}
      """
    And the node "ListAll" uses the "redis" credential "Test Redis"
    And the connections "Start -> SetA -> ListAll"
    When I execute the workflow
    Then the execution succeeds
    And the node "ListAll" outputs:
      """
      [{"keys": ["bdd:redis:onlykey:a"]}]
      """
    And redis has no keys matching "bdd:redis:onlykey:*"

  Scenario: Delete removes a key
    Given redis has no keys matching "bdd:redis:delete:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Set    | redis         |
      | Delete | redis         |
      | Get    | redis         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:delete:1", "value": "gone soon", "keyType": "string"}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Delete" has parameters:
      """
      {"operation": "delete", "key": "bdd:redis:delete:1"}
      """
    And the node "Delete" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:delete:1", "keyType": "automatic", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Delete -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"value": null}]
      """

  Scenario: Info returns the server's generic information
    Given the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Info  | redis         |
    And the node "Info" has parameters:
      """
      {"operation": "info"}
      """
    And the node "Info" uses the "redis" credential "Test Redis"
    And the connections "Start -> Info"
    When I execute the workflow
    Then the execution succeeds
    And the node "Info" outputs items matching:
      """
      [{"redis_version": "$string", "tcp_port": "$number"}]
      """

  Scenario: Getting a missing key with automatic type detection returns null
    Given redis has no keys matching "bdd:redis:missing:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Get   | redis         |
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:missing:does-not-exist", "keyType": "automatic", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"value": null}]
      """

  Scenario: Reading a string key as a hash fails with a wrong-type error
    Given redis has no keys matching "bdd:redis:wrongtype:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | redis         |
      | Get   | redis         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:wrongtype:1", "value": "just a string", "keyType": "string"}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:wrongtype:1", "keyType": "hash", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Get"
    When I execute the workflow
    Then the execution fails
    And the node "Get" failed with an error containing "WRONGTYPE"
    And redis has no keys matching "bdd:redis:wrongtype:*"

  @security
  Scenario: A connection failure produces a clear error without leaking the password
    Given the credential "Bad Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379, "password": "sUp3rS3cr3tPassphrase!"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Get   | redis         |
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:conn:whatever", "keyType": "string", "propertyName": "value"}
      """
    And the node "Get" uses the "redis" credential "Bad Redis"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the node "Get" failed with an error containing "Redis"
    And the execution data does not contain "sUp3rS3cr3tPassphrase!"

  Scenario: onError "continueRegularOutput" passes an error item downstream instead of failing
    Given redis has no keys matching "bdd:redis:continue:*"
    And the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name  | type          | onError                |
      | Start | manualTrigger |                         |
      | Set   | redis         |                         |
      | Get   | redis         | continueRegularOutput   |
      | After | noOp          |                         |
    And the node "Set" has parameters:
      """
      {"operation": "set", "key": "bdd:redis:continue:1", "value": "a string", "keyType": "string"}
      """
    And the node "Set" uses the "redis" credential "Test Redis"
    And the node "Get" has parameters:
      """
      {"operation": "get", "key": "bdd:redis:continue:1", "keyType": "hash", "propertyName": "value", "options": {}}
      """
    And the node "Get" uses the "redis" credential "Test Redis"
    And the connections "Start -> Set -> Get -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """
    And redis has no keys matching "bdd:redis:continue:*"

  Scenario: Publish succeeds even with no subscribers listening
    Given the credential "Test Redis" of type "redis" with the data:
      """
      {"host": "127.0.0.1", "port": 6379}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Publish | redis         |
    And the node "Publish" has parameters:
      """
      {"operation": "publish", "channel": "bdd:redis:channel:1", "messageData": "hello subscribers"}
      """
    And the node "Publish" uses the "redis" credential "Test Redis"
    And the connections "Start -> Publish"
    When I execute the workflow
    Then the execution succeeds
