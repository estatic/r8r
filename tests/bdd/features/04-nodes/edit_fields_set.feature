@spec-6.6 @phase-1 @node-set
Feature: Edit Fields (Set) node
  Set v3.4 assigns typed fields to each item, optionally keeping the other
  fields, supports dot notation, and has a raw JSON mode.

  Background:
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Set   | set           |
    And the connections "Start -> Set"
    And the trigger outputs the items:
      """
      [{"id": 7, "name": "Ada", "email": "ada@example.com"}]
      """

  Scenario: Only the assigned fields are output by default
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "options": {}, "assignments": {"assignments": [
        {"id": "1", "name": "customerId", "value": "={{ $json.id }}", "type": "number"},
        {"id": "2", "name": "vip", "value": true, "type": "boolean"}
      ]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"customerId": 7, "vip": true}]
      """

  Scenario: includeOtherFields keeps the input fields
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": true, "options": {}, "assignments": {"assignments": [
        {"id": "1", "name": "vip", "value": true, "type": "boolean"}
      ]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"id": 7, "name": "Ada", "email": "ada@example.com", "vip": true}]
      """

  Scenario: Assignment types convert values
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": false, "options": {}, "assignments": {"assignments": [
        {"id": "1", "name": "asString", "value": "={{ $json.id }}", "type": "string"},
        {"id": "2", "name": "asNumber", "value": "42", "type": "number"},
        {"id": "3", "name": "asBoolean", "value": "true", "type": "boolean"},
        {"id": "4", "name": "asArray", "value": "[1, 2]", "type": "array"},
        {"id": "5", "name": "asObject", "value": "{\"k\": \"v\"}", "type": "object"}
      ]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"asString": "7", "asNumber": 42, "asBoolean": true, "asArray": [1, 2], "asObject": {"k": "v"}}]
      """

  Scenario: A value that does not convert fails the node
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": false, "options": {}, "assignments": {"assignments": [
        {"id": "1", "name": "n", "value": "not a number", "type": "number"}
      ]}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Set" failed with an error containing "expects a number"

  Scenario: Dot notation creates nested fields
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": false, "options": {}, "assignments": {"assignments": [
        {"id": "1", "name": "customer.name", "value": "={{ $json.name }}", "type": "string"},
        {"id": "2", "name": "customer.contact.email", "value": "={{ $json.email }}", "type": "string"}
      ]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"customer": {"name": "Ada", "contact": {"email": "ada@example.com"}}}]
      """

  Scenario: Dot notation can be turned off
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": false, "options": {"dotNotation": false}, "assignments": {"assignments": [
        {"id": "1", "name": "a.b", "value": "x", "type": "string"}
      ]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"a.b": "x"}]
      """

  Scenario: Raw JSON mode builds the item from a JSON template
    Given the node "Set" has parameters:
      """
      {"mode": "raw", "jsonOutput": "={\n  \"id\": {{ $json.id }},\n  \"label\": \"{{ $json.name }} <{{ $json.email }}>\"\n}", "options": {}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"id": 7, "label": "Ada <ada@example.com>"}]
      """

  Scenario: Include only selected input fields
    Given the node "Set" has parameters:
      """
      {"mode": "manual", "includeOtherFields": true, "include": "selected", "includeFields": "name", "options": {},
       "assignments": {"assignments": [{"id": "1", "name": "vip", "value": true, "type": "boolean"}]}}
      """
    When I execute the workflow
    Then the node "Set" outputs:
      """
      [{"name": "Ada", "vip": true}]
      """
