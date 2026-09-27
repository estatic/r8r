@r8r-only @legacy-api
Feature: Tool library for AI agents
  Tools are defined once and referenced by id from agents. Names must suit
  LLM function calling; only safe node types are allowed; a tool in use
  cannot be deleted.

  Background:
    Given a running r8r server
    And I am logged in to the legacy r8r API as "tools@example.com"

  Scenario: Create, list and edit a tool
    When I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "web_search", "description": "Search the web", "node_type": "core.httpRequest",
       "argument_schema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
       "parameters": {"method": "GET", "url": "https://s.example/?q={{ encodeURIComponent($args.q) }}"}}
      """
    Then the response status is 201
    And I remember the response JSON at "id" as "tool"
    When I send a GET request to "/rest/r8r/tools"
    Then the response JSON at "[0].name" is "web_search"
    And the response JSON at "[0].used_by" is 0
    When I send a PATCH request to "/rest/r8r/tools/%{tool}" with body:
      """
      {"description": "Search the whole web"}
      """
    Then the response status is 200
    And the response JSON at "description" is "Search the whole web"

  Scenario Outline: Invalid tools are rejected
    When I send a POST request to "/rest/r8r/tools" with body:
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
    Given I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "dup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    When I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "dup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    Then the response status is 409

  Scenario: A tool used by an agent cannot be deleted
    Given I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "lookup", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    And I remember the response JSON at "id" as "tool"
    And I send a POST request to "/rest/r8r/workflows" with body:
      """
      {"name": "agent-wf", "nodes": [{"id": "a", "node_type": "ai.agent", "position": [0, 0], "parameters": {"tool_ids": ["%{tool}"]}}], "connections": []}
      """
    When I send a DELETE request to "/rest/r8r/tools/%{tool}"
    Then the response status is 409
    And the response JSON at "error" is "tool is in use"
    And the response JSON at "workflows[0].name" is "agent-wf"

  Scenario: A credential used only by a tool cannot be deleted
    Given I send a POST request to "/rest/r8r/credentials" with body:
      """
      {"name": "api", "credential_type": "bearerToken", "data": {"token": "t"}}
      """
    And I remember the response JSON at "id" as "cred"
    And I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "secured", "description": "d", "node_type": "core.httpRequest", "argument_schema": {"type": "object"},
       "parameters": {"url": "https://x", "auth": {"type": "bearer", "credential_id": "%{cred}"}}}
      """
    When I send a DELETE request to "/rest/r8r/credentials/%{cred}"
    Then the response status is 409
    And the response JSON at "tools[0].name" is "secured"

  Scenario: Deleting an unused tool
    Given I send a POST request to "/rest/r8r/tools" with body:
      """
      {"name": "temp", "description": "d", "node_type": "core.code", "argument_schema": {"type": "object"}, "parameters": {"script": "return []"}}
      """
    And I remember the response JSON at "id" as "tool"
    When I send a DELETE request to "/rest/r8r/tools/%{tool}"
    Then the response status is 204
    When I send a GET request to "/rest/r8r/tools/%{tool}"
    Then the response status is 404
