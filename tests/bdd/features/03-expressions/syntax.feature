@spec-2.4 @spec-6.4 @phase-1
Feature: Expression syntax
  A parameter string that starts with `=` is an expression. `{{ … }}`
  blocks are evaluated as JavaScript. A string that is exactly one block
  keeps the value's native type; mixed literal text and blocks produce a
  string. Strings without the leading `=` are literals.

  Scenario Outline: A single {{ }} block keeps its native type
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                    | result            |
      | ={{ 1 + 2 }}                  | 3                 |
      | ={{ 0.1 * 3 }}                | 0.30000000000000004 |
      | ={{ 'a' + 'b' }}              | "ab"              |
      | ={{ true && !false }}         | true              |
      | ={{ [1, 2, 3].map(n => n * 2) }} | [2, 4, 6]      |
      | ={{ ({ a: 1, b: [true] }) }}  | {"a": 1, "b": [true]} |
      | ={{ null }}                   | null              |
      | ={{ 10 / 4 }}                 | 2.5               |

  Scenario Outline: Mixed text and blocks produce a string
    Given the input item:
      """
      {"name": "Ada", "count": 3}
      """
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                                   | result                  |
      | =Hello {{ $json.name }}!                     | "Hello Ada!"            |
      | =Count: {{ $json.count }}                    | "Count: 3"              |
      | ={{ $json.name }} has {{ $json.count }} items | "Ada has 3 items"      |
      | =no blocks at all                            | "no blocks at all"      |
      | =  padded {{ 1 }}                            | "  padded 1"            |

  Scenario: A string without the = prefix is a literal, braces and all
    When I evaluate the expression "{{ 1 + 1 }}"
    Then the result is "{{ 1 + 1 }}"

  Scenario: Multi-line expressions are allowed
    Given the input item:
      """
      {"price": 10, "qty": 3}
      """
    When I evaluate the expression:
      """
      ={{ (() => {
        const subtotal = $json.price * $json.qty;
        return subtotal * 1.2;
      })() }}
      """
    Then the result is 36

  Scenario: A syntax error fails the node and names the problem
    When I evaluate the expression "={{ 1 + }}"
    Then the expression fails

  Scenario: Reading a property of undefined fails with a helpful message
    Given the input item:
      """
      {"user": {}}
      """
    When I evaluate the expression "={{ $json.user.address.city }}"
    Then the expression fails with an error containing "city"

  Scenario: A missing field is undefined, not an error
    Given the input item:
      """
      {"a": 1}
      """
    When I evaluate the expression "={{ $json.missing }}"
    Then the expression has no result
