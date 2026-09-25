@spec-6.4 @spec-8.2 @security @phase-1
Feature: Expression sandbox
  Expressions run in a capability-limited VM (goal G6). They can read the
  data proxy and nothing else: no Function-constructor escapes, no
  `process`, `require`, timers or network, a heap cap, and a per-expression
  time limit (1 s by default).

  Scenario Outline: Host capabilities do not exist in expressions
    When I evaluate the expression "<expression>"
    Then the result is "undefined"

    Examples:
      | expression                     |
      | ={{ typeof process }}          |
      | ={{ typeof require }}          |
      | ={{ typeof setTimeout }}       |
      | ={{ typeof setInterval }}      |
      | ={{ typeof fetch }}            |
      | ={{ typeof XMLHttpRequest }}   |
      | ={{ typeof globalThis.Deno }}  |

  Scenario: The Function constructor cannot reach the host
    When I evaluate the expression "={{ (() => { try { return typeof (function(){}).constructor('return process')() } catch (e) { return 'blocked' } })() }}"
    Then the result is "blocked"

  Scenario: Constructor chains on data objects cannot reach the host
    Given the input item:
      """
      {"a": 1}
      """
    When I evaluate the expression "={{ (() => { try { return typeof $json.constructor.constructor('return this.process')() } catch (e) { return 'blocked' } })() }}"
    Then the result is "blocked"

  Scenario: An expression cannot alter the data of other items or nodes
    Given the input items:
      """
      [{"v": 1}, {"v": 2}]
      """
    When I evaluate the expression "={{ (() => { try { $input.all()[1].json.v = 999 } catch (e) {} return $json.v })() }}"
    Then the results for each item are [1, 2]

  Scenario: A runaway expression is stopped by the time limit
    When I evaluate the expression "={{ (() => { while (true) {} })() }}"
    Then the expression fails
    And the command finished within 10000 ms

  Scenario: An expression that allocates without bound is stopped
    When I evaluate the expression "={{ (() => { let s = 'x'; while (true) { s = s + s; } })() }}"
    Then the expression fails
    And the command finished within 10000 ms

  Scenario: Deep recursion fails the node instead of crashing the process
    When I evaluate the expression "={{ (function f(n) { return f(n + 1) })(0) }}"
    Then the expression fails
