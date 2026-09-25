@n8n-compat
Feature: n8n expressions (spec 2.4, 6.4)
  Parameters starting with "=" are templates whose {{ }} blocks run as
  JavaScript against n8n's data proxy. A lone {{ }} keeps its native type;
  mixed text stringifies. Expressions are sandboxed and time-limited.

  Background:
    Given an r8n instance with an owner account
    And I have a public API key

  Scenario Outline: Values and types
    When I evaluate the expression "<expression>"
    Then the expression result is:
      """
      <result>
      """

    Examples:
      | expression               | result          |
      | 1 + 1                    | 2               |
      | 'a' + 'b'                | "ab"            |
      | [1, 2, 3].length         | 3               |
      | ({ x: 1 })               | {"x": 1}        |
      | null                     | null            |
      | true && false            | false           |

  Scenario Outline: The data proxy
    When I evaluate the expression "<expression>" against:
      """
      {"name": "Ada", "tags": ["a", "b"], "nested": {"deep": {"n": 7}}}
      """
    Then the expression result is:
      """
      <result>
      """

    Examples:
      | expression                     | result          |
      | $json.name                     | "Ada"           |
      | $json.nested.deep.n            | 7               |
      | $json.tags[1]                  | "b"             |
      | $input.first().json.name       | "Ada"           |
      | $input.all().length            | 1               |
      | $('Trigger').item.json.name    | "Ada"           |
      | $json.missing                  | null            |
      | $workflow.name                 | "evaluate"      |

  Scenario Outline: n8n extension methods (documented examples)
    When I evaluate the expression "<expression>"
    Then the expression result is:
      """
      <result>
      """

    Examples:
      | expression                                          | result                 |
      | 'hello world'.toSnakeCase()                         | "hello_world"          |
      | 'Hello World'.toCamelCase()                         | "helloWorld"           |
      | 'contact me at ada@example.com'.extractEmail()      | "ada@example.com"      |
      | 'abc'.isEmpty()                                     | false                  |
      | [1, 1, 2, 3, 3].unique()                            | [1, 2, 3]              |
      | [{ a: 1 }, { a: 2 }].pluck('a')                     | [1, 2]                 |
      | [3, 1, 2].max()                                     | 3                      |
      | (1234.5678).round(2)                                | 1234.57                |

  Scenario Outline: Luxon dates
    When I evaluate the expression "<expression>"
    Then the expression result is:
      """
      <result>
      """

    Examples:
      | expression                                                                 | result          |
      | DateTime.fromISO('2024-01-31').plus({ days: 1 }).toISODate()                | "2024-02-01"    |
      | DateTime.fromISO('2024-03-10T12:00:00Z').toFormat('yyyy-LL-dd')             | "2024-03-10"    |
      | typeof $now.toISO()                                                        | "string"        |
      | $today.hour                                                                | 0               |

  Scenario: Mixed text and expressions produce a string
    Given the n8n workflow "mixed":
      """
      {"nodes": [
        {"id": "1", "name": "Trigger", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
        {"id": "2", "name": "Set", "type": "n8n-nodes-base.set", "typeVersion": 3.4, "position": [200, 0],
         "parameters": {"assignments": {"assignments": [
           {"id": "a", "name": "mixed", "value": "=n is {{ 1 + 1 }}!", "type": "string"},
           {"id": "b", "name": "literal", "value": "{{ 1 + 1 }}", "type": "string"}
         ]}, "options": {}}}
       ],
       "connections": {"Trigger": {"main": [[{"node": "Set", "type": "main", "index": 0}]]}}}
      """
    When I run the workflow "mixed" manually
    Then the execution status is "success"
    And node "Set" output 0 item 0 has JSON:
      """
      {"mixed": "n is 2!", "literal": "{{ 1 + 1 }}"}
      """

  Scenario Outline: Sandbox escapes are refused
    When I evaluate the expression "<expression>"
    Then the expression fails with an error containing "<error>"

    Examples:
      | expression                                                  | error          |
      | this.constructor.constructor('return process')().pid         | not allowed    |
      | Function('return process')()                                | not allowed    |
      | require('fs')                                               | not allowed    |
      | process.env                                                 | not allowed    |
      | setTimeout(() => 1, 10)                                     | not allowed    |
      | Object.getPrototypeOf($json).constructor('return 1')()      | not allowed    |

  Scenario: Environment variables are blocked by default
    When I evaluate the expression "$env.HOME"
    Then the expression fails with an error containing "access to env vars denied"

  Scenario: A runaway expression is stopped
    When I evaluate the expression "(() => { while (true) {} })()"
    Then the expression fails with an error containing "timed out"

  Scenario: Syntax errors are reported, not crashed on
    When I evaluate the expression "1 +"
    Then the expression fails with an error containing "invalid syntax"

  @needs-n8n-fixture
  Scenario: Differential corpus matches n8n
    Given the recorded n8n expression corpus
    When every corpus expression is evaluated in r8n
    Then every result equals n8n's recorded result
