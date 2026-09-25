@r8r
Feature: Tool library for AI agents
  Tools are defined once and referenced by id from agents. Names must suit
  LLM function calling; only safe node types are allowed; a tool in use
  cannot be deleted.

  Background:
    Given I am logged in as "tools@example.com"

  Scenario: Create, list and edit a tool
    When I send a POST request to "/rest/tools" with JSON:
      """
      {"name": "web_search", "description": "Search the web", "node_type": "core.httpRequest",
       "argument_schema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
       "parameters": {"method": "GET", "url": "https://s.example/?q={{ encodeURIComponent($args.q) }}"}}
      """
    Then the response status is 201
    And I remember the JSON at "/id" as "tool"
    When I send a GET request to "/rest/tools"
    Then the JSON at "/0/name" is "web_search"
    And the JSON at "/0/used_by" is 0
    When I send a PATCH request to "/rest/tools/{tool}" with JSON:
      """
      {"description": "Search the whole web"}
      """
    Then the response status is 200
    And the JSON at "/description" is "Search the whole web"

  Scenario Outline: Invalid tools are rejected
    When I send a POST request to "/rest/tools" with JSON:
      """
      {"name": "<name>", "description": "d", "node_type": "<node_type>", "argument_schema": <schema>, "parameters": <parameters>}
      """
    Then the response status is 400

    Examples:
      | name       | node_type        | schema                                                                 | parameters |
      | has space  | core.code        | {"type": "object", "properties": {}}                                   | {}         |
      | ok_name    | ai.agent         | {"type": "object", "properties": {}}                                   | {}         |
      | ok_name    | core.set         | {"type": "object", "properties": {}}                                   | {}         |
      | ok_name    | core.code        | {"type": "array"}                                                      | {}         |
      | ok_name    | core.code        | {"type": "object", "properties": {"x": {"type": "object"}}}            | {}         |
      | ok_name    | core.code        | {"type": "object", "properties": {}, "required": ["ghost"]}            | {}         |
      | ok_name    | core.code        | {"type": "object", "properties": {}}                                   | "text"     |

  Scenario: Tool names are unique
    Given I sent a POST request to "/rest/tools" with JSON:
      """
      {"name": "dup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    When I send a POST request to "/rest/tools" with JSON:
      """
      {"name": "dup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    Then the response status is 409

  Scenario: A tool used by an agent cannot be deleted
    Given I sent a POST request to "/rest/tools" with JSON:
      """
      {"name": "lookup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    And I remember the JSON at "/id" as "tool"
    And I sent a POST request to "/rest/workflows" with JSON:
      """
      {"name": "agent-wf", "nodes": [{"id": "a", "node_type": "ai.agent", "position": [0, 0], "parameters": {"tool_ids": ["{tool}"]}}], "connections": []}
      """
    When I send a DELETE request to "/rest/tools/{tool}"
    Then the response status is 409
    And the JSON at "/error" is "tool is in use"
    And the JSON at "/workflows/0/name" is "agent-wf"

  Scenario: A credential used only by a tool cannot be deleted
    Given I sent a POST request to "/rest/credentials" with JSON:
      """
      {"name": "api", "credential_type": "bearerToken", "data": {"token": "t"}}
      """
    And I remember the JSON at "/id" as "cred"
    And I sent a POST request to "/rest/tools" with JSON:
      """
      {"name": "secured", "description": "d", "node_type": "core.httpRequest", "argument_schema": {"type": "object"},
       "parameters": {"url": "https://x", "auth": {"type": "bearer", "credential_id": "{cred}"}}}
      """
    When I send a DELETE request to "/rest/credentials/{cred}"
    Then the response status is 409
    And the JSON at "/tools/0/name" is "secured"

  Scenario: Deleting an unused tool
    Given I sent a POST request to "/rest/tools" with JSON:
      """
      {"name": "temp", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    And I remember the JSON at "/id" as "tool"
    When I send a DELETE request to "/rest/tools/{tool}"
    Then the response status is 204
    When I send a GET request to "/rest/tools/{tool}"
    Then the response status is 404
