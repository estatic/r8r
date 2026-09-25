@spec-6.6 @phase-1 @node-if @node-filter @node-switch
Feature: If, Filter and Switch route items by conditions
  Conditions use n8n's filter structure (typed operators, combinator,
  case sensitivity, strict type validation) and are evaluated per item.

  Background:
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Route  | if            |
      | Yes    | noOp          |
      | No     | noOp          |
    And the connections:
      """
      Start -> Route
      Route:0 -> Yes
      Route:1 -> No
      """
    And the trigger outputs the items:
      """
      [{"amount": 50, "country": "DE", "email": "a@x.io"}, {"amount": 500, "country": "de", "email": ""}, {"amount": 150, "country": "FR", "email": "c@y.io"}]
      """

  Scenario: If routes each item to true or false
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ $json.amount }}", "rightValue": 100, "operator": {"type": "number", "operation": "gt"}}],
        "combinator": "and"}, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" is:
      """
      [{"amount": 500, "country": "de", "email": ""}, {"amount": 150, "country": "FR", "email": "c@y.io"}]
      """
    And output 1 of the node "Route" is:
      """
      [{"amount": 50, "country": "DE", "email": "a@x.io"}]
      """

  Scenario: Conditions combined with "and"
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [
          {"id": "1", "leftValue": "={{ $json.amount }}", "rightValue": 100, "operator": {"type": "number", "operation": "gt"}},
          {"id": "2", "leftValue": "={{ $json.email }}", "rightValue": "", "operator": {"type": "string", "operation": "notEmpty", "singleValue": true}}
        ],
        "combinator": "and"}, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" has 1 item
    And output 0 of the node "Route" has items matching:
      """
      [{"amount": 150}]
      """

  Scenario: Conditions combined with "or"
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [
          {"id": "1", "leftValue": "={{ $json.amount }}", "rightValue": 400, "operator": {"type": "number", "operation": "gt"}},
          {"id": "2", "leftValue": "={{ $json.country }}", "rightValue": "FR", "operator": {"type": "string", "operation": "equals"}}
        ],
        "combinator": "or"}, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" has 2 items
    And output 1 of the node "Route" has 1 item

  Scenario Outline: String comparison honours caseSensitive
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": <caseSensitive>, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ $json.country }}", "rightValue": "DE", "operator": {"type": "string", "operation": "equals"}}],
        "combinator": "and"}, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" has <matches> items

    Examples:
      | caseSensitive | matches |
      | true          | 1       |
      | false         | 2       |

  Scenario: Strict type validation rejects a string compared as a number
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ $json.country }}", "rightValue": 1, "operator": {"type": "number", "operation": "gt"}}],
        "combinator": "and"}, "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Route" failed with an error containing "Wrong type"

  Scenario: Loose type validation converts where it can
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "loose", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ String($json.amount) }}", "rightValue": 100, "operator": {"type": "number", "operation": "gt"}}],
        "combinator": "and"}, "looseTypeValidation": true, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" has 2 items

  Scenario Outline: Operators
    Given the node "Route" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "<left>", "rightValue": <right>, "operator": <operator>}],
        "combinator": "and"}, "options": {}}
      """
    When I execute the workflow
    Then output 0 of the node "Route" has <matches> items

    Examples:
      | left                    | right | operator                                                              | matches |
      | ={{ $json.email }}      | "@"   | {"type": "string", "operation": "contains"}                           | 2       |
      | ={{ $json.email }}      | ".io" | {"type": "string", "operation": "endsWith"}                           | 2       |
      | ={{ $json.email }}      | "^c@" | {"type": "string", "operation": "regex"}                              | 1       |
      | ={{ $json.email }}      | ""    | {"type": "string", "operation": "empty", "singleValue": true}         | 1       |
      | ={{ $json.amount }}     | 150   | {"type": "number", "operation": "lte"}                                | 2       |
      | ={{ $json.amount }}     | 500   | {"type": "number", "operation": "equals"}                             | 1       |
      | ={{ $json.missing }}    | ""    | {"type": "string", "operation": "exists", "singleValue": true}        | 0       |
      | ={{ [1, 2] }}           | 2     | {"type": "array", "operation": "contains", "rightType": "any"}        | 3       |
      | ={{ $json.amount > 100 }} | ""  | {"type": "boolean", "operation": "true", "singleValue": true}         | 2       |

  Scenario: Filter keeps matching items and drops the rest
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Keep   | filter        |
    And the node "Keep" has parameters:
      """
      {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
        "conditions": [{"id": "1", "leftValue": "={{ $json.active }}", "rightValue": "", "operator": {"type": "boolean", "operation": "true", "singleValue": true}}],
        "combinator": "and"}, "options": {}}
      """
    And the connections "Start -> Keep"
    And the trigger outputs the items:
      """
      [{"id": 1, "active": true}, {"id": 2, "active": false}, {"id": 3, "active": true}]
      """
    When I execute the workflow
    Then the node "Keep" outputs:
      """
      [{"id": 1, "active": true}, {"id": 3, "active": true}]
      """

  Scenario: Switch in rules mode sends each item to the first matching rule's output
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Priority | switch        |
    And the node "Priority" has parameters:
      """
      {"mode": "rules", "rules": {"values": [
        {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
          "conditions": [{"id": "1", "leftValue": "={{ $json.priority }}", "rightValue": "high", "operator": {"type": "string", "operation": "equals"}}], "combinator": "and"}},
        {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
          "conditions": [{"id": "2", "leftValue": "={{ $json.priority }}", "rightValue": "low", "operator": {"type": "string", "operation": "equals"}}], "combinator": "and"}}
      ]}, "options": {"fallbackOutput": "extra"}}
      """
    And the connections "Start -> Priority"
    And the trigger outputs the items:
      """
      [{"priority": "low"}, {"priority": "high"}, {"priority": "unknown"}]
      """
    When I execute the workflow
    Then output 0 of the node "Priority" is:
      """
      [{"priority": "high"}]
      """
    And output 1 of the node "Priority" is:
      """
      [{"priority": "low"}]
      """
    And output 2 of the node "Priority" is:
      """
      [{"priority": "unknown"}]
      """

  Scenario: Switch with allMatchingOutputs sends an item to every matching output
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Fanout | switch        |
    And the node "Fanout" has parameters:
      """
      {"mode": "rules", "rules": {"values": [
        {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
          "conditions": [{"id": "1", "leftValue": "={{ $json.n }}", "rightValue": 0, "operator": {"type": "number", "operation": "gt"}}], "combinator": "and"}},
        {"conditions": {"options": {"caseSensitive": true, "leftValue": "", "typeValidation": "strict", "version": 2},
          "conditions": [{"id": "2", "leftValue": "={{ $json.n }}", "rightValue": 10, "operator": {"type": "number", "operation": "gt"}}], "combinator": "and"}}
      ]}, "options": {"allMatchingOutputs": true}}
      """
    And the connections "Start -> Fanout"
    And the trigger outputs the items:
      """
      [{"n": 50}]
      """
    When I execute the workflow
    Then output 0 of the node "Fanout" has 1 item
    And output 1 of the node "Fanout" has 1 item

  Scenario: Switch in expression mode picks the output by index
    Given a workflow with nodes:
      | name   | type          | parameters                                                             |
      | Start  | manualTrigger |                                                                        |
      | ByIdx  | switch        | {"mode": "expression", "numberOutputs": 3, "output": "={{ $json.lane }}"} |
    And the connections "Start -> ByIdx"
    And the trigger outputs the items:
      """
      [{"lane": 2}, {"lane": 0}]
      """
    When I execute the workflow
    Then output 0 of the node "ByIdx" is:
      """
      [{"lane": 0}]
      """
    And output 1 of the node "ByIdx" is empty
    And output 2 of the node "ByIdx" is:
      """
      [{"lane": 2}]
      """
