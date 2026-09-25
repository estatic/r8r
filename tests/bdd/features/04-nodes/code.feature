@spec-6.7 @phase-3 @node-code
Feature: Code node (JavaScript)
  The Code node runs user JavaScript out of process in `r8r runner`, in two
  modes: once for all items, or once for each item. It sees `$input`,
  `$json`, `items`, `$()` and the other n8n variables, and its console
  output is forwarded to the editor.

  Background:
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the connections "Start -> Code"
    And the trigger outputs the items:
      """
      [{"n": 1}, {"n": 2}, {"n": 3}]
      """

  Scenario: Run once for all items
    Given the node "Code" runs the JavaScript:
      """
      const total = $input.all().reduce((sum, item) => sum + item.json.n, 0);
      return [{ json: { total, count: items.length } }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"total": 6, "count": 3}]
      """

  Scenario: Run once for each item
    Given the node "Code" runs the JavaScript for each item:
      """
      return { json: { n: $json.n, square: $json.n * $json.n, index: $itemIndex } };
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"n": 1, "square": 1, "index": 0}, {"n": 2, "square": 4, "index": 1}, {"n": 3, "square": 9, "index": 2}]
      """

  Scenario: Plain objects are wrapped into items
    Given the node "Code" runs the JavaScript:
      """
      return [{ a: 1 }, { a: 2 }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"a": 1}, {"a": 2}]
      """

  Scenario: Async code is awaited
    Given the node "Code" runs the JavaScript:
      """
      const value = await new Promise(resolve => resolve(42));
      return [{ json: { value } }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"value": 42}]
      """

  Scenario: Earlier nodes are reachable with $()
    Given the node "Code" runs the JavaScript:
      """
      return [{ json: { fromStart: $('Start').all().map(i => i.json.n) } }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"fromStart": [1, 2, 3]}]
      """

  Scenario: Output items are paired with their input items in each-item mode
    Given the node "Code" runs the JavaScript for each item:
      """
      return { json: { doubled: $json.n * 2 } };
      """
    When I execute the workflow
    Then the items of the node "Code" are paired as:
      | item | pairedItem  |
      | 0    | {"item": 0} |
      | 2    | {"item": 2} |

  Scenario: A thrown error fails the node with the error's message and line
    Given the node "Code" runs the JavaScript:
      """
      const x = 1;
      throw new Error('bad input');
      """
    When I execute the workflow
    Then the execution fails
    And the node "Code" failed with an error containing "bad input"
    And the node "Code" failed with an error containing "[line"

  Scenario: Returning something that is not items fails with a clear message
    Given the node "Code" runs the JavaScript:
      """
      return 42;
      """
    When I execute the workflow
    Then the execution fails
    And the node "Code" failed with an error containing "return"

  Scenario: The workflow's static data persists values
    Given the node "Code" runs the JavaScript:
      """
      const data = $getWorkflowStaticData('global');
      data.count = (data.count || 0) + 1;
      return [{ json: { count: data.count } }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"count": 1}]
      """

  Scenario: Built-in modules can be allowed explicitly
    Given the environment variable "NODE_FUNCTION_ALLOW_BUILTIN" is "crypto"
    And the node "Code" runs the JavaScript:
      """
      const crypto = require('crypto');
      return [{ json: { hash: crypto.createHash('sha256').update('abc').digest('hex') } }];
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"hash": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]
      """

  @phase-2
  Scenario: console.log output is forwarded to the editor
    Given a running r8r server with an owner and an API key
    And I am connected to the push channel
    And a workflow named "Logs" with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the node "Code" runs the JavaScript:
      """
      console.log('hello from code');
      return [{ json: {} }];
      """
    And the connections "Start -> Code"
    When I run the workflow manually from the editor
    Then I receive the push messages in order:
      """
      executionStarted
      sendConsoleMessage
      executionFinished
      """

  @requires-python-runner
  Scenario: Python code runs in the external runner
    Given the node "Code" runs the Python:
      """
      return [{"json": {"total": sum(item.json.n for item in _items)}}]
      """
    When I execute the workflow
    Then the node "Code" outputs:
      """
      [{"total": 6}]
      """
