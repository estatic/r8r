@spec-6.6 @phase-4 @node-notion
Feature: Notion node
  Consumes the Notion REST API (v2/2.2, as the n8n 2.35.7 editor creates the
  node) against the `notionApi` (Internal Integration Secret) credential.
  `notionOAuth2Api` authentication and Notion Trigger are out of scope;
  anything unimplemented fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "Integration" of type "notionApi" with the data:
      """
      {"apiKey": "secret_test_token", "url": "%{MOCK_URL}"}
      """

  # ---- block -----------------------------------------------------------

  Scenario: Appending blocks sends formatted children and the Notion-Version header
    Given the mock service responds to PATCH "/v1/blocks/parent1/children" with status 200 and body:
      """
      {"object": "list", "results": [{"object": "block", "id": "b1", "type": "paragraph"}]}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Append | notion |
    And the node "Append" has parameters:
      """
      {
        "resource": "block",
        "operation": "append",
        "blockId": {"mode": "id", "value": "parent1"},
        "blockUi": {"blockValues": [
          {"type": "paragraph", "richText": false, "textContent": "Hello world"},
          {"type": "to_do", "richText": false, "textContent": "Buy milk", "checked": true},
          {"type": "image", "url": "https://example.com/pic.png"}
        ]}
      }
      """
    And the node "Append" uses the "notionApi" credential "Integration"
    And the connections "Start -> Append"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/blocks/parent1/children" had the header "authorization" equal to "Bearer secret_test_token"
    And the last request to "/v1/blocks/parent1/children" had the header "notion-version" equal to "2021-08-16"
    And the last request to "/v1/blocks/parent1/children" had a JSON body matching:
      """
      {
        "children": [
          {"object": "block", "type": "paragraph", "paragraph": {"text": [{"text": {"content": "Hello world"}}]}},
          {"object": "block", "type": "to_do", "to_do": {"checked": true, "text": [{"text": {"content": "Buy milk"}}]}},
          {"object": "block", "type": "image", "image": {"type": "external", "external": {"url": "https://example.com/pic.png"}}}
        ]
      }
      """
    And the node "Append" outputs:
      """
      [{"object": "list", "results": [{"object": "block", "id": "b1", "type": "paragraph"}]}]
      """

  Scenario: Getting many blocks with a limit sends page_size and stops paging once the limit is reached
    Given the mock service responds to GET "/v1/blocks/parent1/children" in order with:
      """
      [
        {"status": 200, "body": {"results": [{"object": "block", "id": "b1"}, {"object": "block", "id": "b2"}], "has_more": true, "next_cursor": "cursor-2"}},
        {"status": 200, "body": {"results": [{"object": "block", "id": "b1"}, {"object": "block", "id": "b2"}], "has_more": true, "next_cursor": "cursor-3"}}
      ]
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {
        "resource": "block",
        "operation": "getAll",
        "blockId": {"mode": "id", "value": "parent1"},
        "returnAll": false,
        "limit": 3,
        "fetchNestedBlocks": false
      }
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/v1/blocks/parent1/children"
    And the last request to "/v1/blocks/parent1/children" had the query parameter "page_size" equal to "3"
    And the node "GetMany" outputs 3 items

  Scenario: Getting all blocks paginates until has_more is false and fetches nested children
    Given the mock service responds to GET "/v1/blocks/root1/children" in order with:
      """
      [
        {"status": 200, "body": {"results": [{"object": "block", "id": "b1", "type": "paragraph", "has_children": true}], "has_more": false}}
      ]
      """
    And the mock service responds to GET "/v1/blocks/b1/children" with status 200 and body:
      """
      {"results": [{"object": "block", "id": "b1a", "type": "paragraph", "has_children": false}], "has_more": false}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | GetAll | notion |
    And the node "GetAll" has parameters:
      """
      {
        "resource": "block",
        "operation": "getAll",
        "blockId": {"mode": "id", "value": "root1"},
        "returnAll": true,
        "fetchNestedBlocks": true
      }
      """
    And the node "GetAll" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetAll"
    When I execute the workflow
    Then the execution succeeds
    And the node "GetAll" outputs:
      """
      [
        {"object": "block", "id": "b1", "type": "paragraph", "has_children": true, "parent_id": "root1"},
        {"object": "block", "id": "b1a", "type": "paragraph", "has_children": false, "parent_id": "b1"}
      ]
      """

  # ---- database ----------------------------------------------------------

  Scenario: Getting a database simplifies the response by default
    Given the mock service responds to GET "/v1/databases/db1" with status 200 and body:
      """
      {"object": "database", "id": "db1", "url": "https://notion.so/db1", "title": [{"plain_text": "Tasks"}]}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "database", "operation": "get", "databaseId": {"mode": "id", "value": "db1"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": "db1", "name": "Tasks", "url": "https://notion.so/db1"}]
      """

  Scenario: Getting a database with simple off returns the raw object
    Given the mock service responds to GET "/v1/databases/db1" with status 200 and body:
      """
      {"object": "database", "id": "db1", "url": "https://notion.so/db1", "title": [{"plain_text": "Tasks"}]}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "database", "operation": "get", "databaseId": {"mode": "id", "value": "db1"}, "simple": false}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"object": "database", "id": "db1", "url": "https://notion.so/db1", "title": [{"plain_text": "Tasks"}]}]
      """

  Scenario: Getting many databases searches by object type and paginates while returnAll is set
    Given the mock service responds to POST "/v1/search" in order with:
      """
      [
        {"status": 200, "body": {"results": [{"object": "database", "id": "db1", "title": [{"plain_text": "A"}]}], "has_more": true, "next_cursor": "c2"}},
        {"status": 200, "body": {"results": [{"object": "database", "id": "db2", "title": [{"plain_text": "B"}]}], "has_more": false}}
      ]
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {"resource": "database", "operation": "getAll", "returnAll": true, "simple": true}
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/v1/search"
    And the last request to "/v1/search" had a JSON body matching:
      """
      {"filter": {"property": "object", "value": "database"}, "start_cursor": "c2"}
      """
    And the node "GetMany" outputs:
      """
      [{"id": "db1", "name": "A"}, {"id": "db2", "name": "B"}]
      """

  Scenario: Searching databases sends the search text and sort
    Given the mock service responds to POST "/v1/search" with status 200 and body:
      """
      {"results": [{"object": "database", "id": "db1", "title": [{"plain_text": "Tasks"}]}], "has_more": false}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Search | notion |
    And the node "Search" has parameters:
      """
      {
        "resource": "database",
        "operation": "search",
        "text": "Tasks",
        "returnAll": false,
        "limit": 10,
        "simple": true,
        "options": {"sort": {"sortValue": {"direction": "ascending", "timestamp": "last_edited_time"}}}
      }
      """
    And the node "Search" uses the "notionApi" credential "Integration"
    And the connections "Start -> Search"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/search" had a JSON body matching:
      """
      {
        "filter": {"property": "object", "value": "database"},
        "query": "Tasks",
        "sort": {"direction": "ascending", "timestamp": "last_edited_time"}
      }
      """
    And the node "Search" outputs:
      """
      [{"id": "db1", "name": "Tasks"}]
      """

  # ---- databasePage --------------------------------------------------------

  Scenario: Creating a database page maps every property type and looks up the title key once
    Given the mock service responds to GET "/v1/databases/db1" with status 200 and body:
      """
      {"properties": {"Name": {"type": "title"}}}
      """
    And the mock service responds to POST "/v1/pages" with status 200 and body:
      """
      {"object": "page", "id": "page1", "url": "https://notion.so/page1", "parent": {"type": "database_id"}, "properties": {}}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Create | notion |
    And the node "Create" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "create",
        "databaseId": {"mode": "id", "value": "db1"},
        "title": "My task",
        "simple": true,
        "propertiesUi": {"propertyValues": [
          {"key": "Status|select", "selectValue": "Done"},
          {"key": "Tags|multi_select", "multiSelectValue": ["Urgent", "Bug"]},
          {"key": "Count|number", "numberValue": 5},
          {"key": "Done|checkbox", "checkboxValue": true},
          {"key": "Link|url", "urlValue": "https://example.com"},
          {"key": "Email|email", "emailValue": "a@example.com"},
          {"key": "Phone|phone_number", "phoneValue": "+1234567890"},
          {"key": "Owner|people", "peopleValue": ["user-1"]},
          {"key": "Related|relation", "relationValue": ["11111111-1111-1111-1111-111111111111"]},
          {"key": "Notes|rich_text", "richText": false, "textContent": "hello"},
          {"key": "Due|date", "date": "2024-01-15", "includeTime": false, "range": false, "timezone": "UTC"},
          {"key": "Attachments|files", "fileUrls": {"fileUrl": [{"name": "doc.pdf", "url": "https://example.com/doc.pdf"}]}}
        ]},
        "blockUi": {"blockValues": []},
        "options": {}
      }
      """
    And the node "Create" uses the "notionApi" credential "Integration"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/pages" had a JSON body matching:
      """
      {
        "parent": {"database_id": "db1"},
        "properties": {
          "Name": {"title": [{"text": {"content": "My task"}}]},
          "Status": {"type": "select", "select": {"name": "Done"}},
          "Tags": {"type": "multi_select", "multi_select": [{"name": "Urgent"}, {"name": "Bug"}]},
          "Count": {"type": "number", "number": 5},
          "Done": {"type": "checkbox", "checkbox": true},
          "Link": {"type": "url", "url": "https://example.com"},
          "Email": {"type": "email", "email": "a@example.com"},
          "Phone": {"type": "phone_number", "phone_number": "+1234567890"},
          "Owner": {"type": "people", "people": [{"id": "user-1"}]},
          "Related": {"type": "relation", "relation": [{"id": "11111111-1111-1111-1111-111111111111"}]},
          "Notes": {"rich_text": [{"text": {"content": "hello"}}]},
          "Due": {"type": "date", "date": {"start": "2024-01-15", "end": null}},
          "Attachments": {"type": "files", "files": [{"name": "doc.pdf", "type": "external", "external": {"url": "https://example.com/doc.pdf"}}]}
        }
      }
      """

  Scenario: Creating a database page uses item 0's database for the title-key lookup even when later items differ
    Given the mock service responds to GET "/v1/databases/db1" with status 200 and body:
      """
      {"properties": {"Name": {"type": "title"}}}
      """
    And the mock service responds to POST "/v1/pages" with status 200 and body:
      """
      {"object": "page", "id": "page1", "properties": {}}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Create | notion |
    And the node "Create" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "create",
        "databaseId": {"mode": "id", "value": "={{ $json.dbId }}"},
        "title": "={{ $json.title }}",
        "simple": true,
        "propertiesUi": {"propertyValues": []},
        "blockUi": {"blockValues": []},
        "options": {}
      }
      """
    And the node "Create" uses the "notionApi" credential "Integration"
    And the connections "Start -> Create"
    And the trigger outputs the items:
      """
      [{"dbId": "db1", "title": "first"}, {"dbId": "db2", "title": "second"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/v1/databases/db1"
    And the mock service received 0 requests to "/v1/databases/db2"
    And the 2nd request to "/v1/pages" had the header "authorization" equal to "Bearer secret_test_token"

  Scenario: Getting a database page simplifies with a property_ prefix
    Given the mock service responds to GET "/v1/pages/page1" with status 200 and body:
      """
      {
        "object": "page",
        "id": "page1",
        "url": "https://notion.so/page1",
        "parent": {"type": "database_id"},
        "properties": {
          "Name": {"type": "title", "title": [{"plain_text": "My task"}]},
          "Status": {"type": "select", "select": {"name": "Done"}}
        }
      }
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": "page1", "name": "My task", "url": "https://notion.so/page1", "property_name": "My task", "property_status": "Done"}]
      """

  Scenario: Getting many database pages with a manual filter sends the mapped Notion filter
    Given the mock service responds to POST "/v1/databases/db1/query" with status 200 and body:
      """
      {"results": [{"object": "page", "id": "p1", "parent": {"type": "database_id"}, "properties": {"Name": {"type": "title", "title": [{"plain_text": "Task 1"}]}}}], "has_more": false}
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "getAll",
        "databaseId": {"mode": "id", "value": "db1"},
        "returnAll": true,
        "simple": true,
        "filterType": "manual",
        "matchType": "anyFilter",
        "filters": {"conditions": [
          {"key": "Status|select", "condition": "equals", "selectValue": "Done"}
        ]},
        "options": {}
      }
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/databases/db1/query" had a JSON body matching:
      """
      {"filter": {"or": [{"property": "Status", "select": {"equals": "Done"}}]}}
      """
    And the node "GetMany" outputs:
      """
      [{"id": "p1", "name": "Task 1", "property_name": "Task 1"}]
      """

  Scenario: Getting many database pages with a JSON filter and sort sends both as-is
    Given the mock service responds to POST "/v1/databases/db1/query" with status 200 and body:
      """
      {"results": [], "has_more": false}
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "getAll",
        "databaseId": {"mode": "id", "value": "db1"},
        "returnAll": false,
        "limit": 10,
        "simple": true,
        "filterType": "json",
        "filterJson": "{\"property\": \"Status\", \"select\": {\"equals\": \"Done\"}}",
        "options": {"sort": {"sortValue": [
          {"key": "Due|date", "direction": "ascending", "timestamp": false},
          {"key": "created_time|created_time", "direction": "descending", "timestamp": true}
        ]}}
      }
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/databases/db1/query" had a JSON body matching:
      """
      {
        "filter": {"property": "Status", "select": {"equals": "Done"}},
        "sorts": [{"direction": "ascending", "property": "Due"}, {"direction": "descending", "timestamp": "created_time"}],
        "page_size": 10
      }
      """

  Scenario: An invalid JSON filter fails with n8n's validation message
    Given a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "getAll",
        "databaseId": {"mode": "id", "value": "db1"},
        "returnAll": true,
        "simple": true,
        "filterType": "json",
        "filterJson": "not valid json",
        "options": {}
      }
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Filters (JSON) must be a valid json"

  Scenario: Updating a database page maps properties and writes a tagged icon
    Given the mock service responds to PATCH "/v1/pages/page1" with status 200 and body:
      """
      {"object": "page", "id": "page1", "parent": {"type": "database_id"}, "properties": {}}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Update | notion |
    And the node "Update" has parameters:
      """
      {
        "resource": "databasePage",
        "operation": "update",
        "pageId": {"mode": "id", "value": "page1"},
        "simple": true,
        "propertiesUi": {"propertyValues": [
          {"key": "Status|select", "selectValue": "Done"}
        ]},
        "options": {"iconType": "emoji", "icon": "🚀"}
      }
      """
    And the node "Update" uses the "notionApi" credential "Integration"
    And the connections "Start -> Update"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/pages/page1" had a JSON body matching:
      """
      {
        "properties": {"Status": {"type": "select", "select": {"name": "Done"}}},
        "icon": {"type": "emoji", "emoji": "🚀"}
      }
      """

  # ---- page ----------------------------------------------------------------

  Scenario: Archiving a page sends archived true
    Given the mock service responds to PATCH "/v1/pages/page1" with status 200 and body:
      """
      {"object": "page", "id": "page1", "parent": {"type": "page_id"}, "properties": {"title": {"title": [{"plain_text": "Old page"}]}}, "url": "https://notion.so/page1"}
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | Archive | notion |
    And the node "Archive" has parameters:
      """
      {"resource": "page", "operation": "archive", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the node "Archive" uses the "notionApi" credential "Integration"
    And the connections "Start -> Archive"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/pages/page1" had a JSON body matching:
      """
      {"archived": true}
      """
    And the node "Archive" outputs:
      """
      [{"id": "page1", "name": "Old page", "url": "https://notion.so/page1"}]
      """

  Scenario: Creating a page under a parent page sends an untagged emoji icon
    Given the mock service responds to POST "/v1/pages" with status 200 and body:
      """
      {"object": "page", "id": "page2", "parent": {"type": "page_id"}, "properties": {"title": {"title": [{"plain_text": "New page"}]}}, "url": "https://notion.so/page2"}
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Create | notion |
    And the node "Create" has parameters:
      """
      {
        "resource": "page",
        "operation": "create",
        "pageId": {"mode": "id", "value": "parent1"},
        "title": "New page",
        "simple": true,
        "blockUi": {"blockValues": []},
        "options": {"iconType": "emoji", "icon": "📄"}
      }
      """
    And the node "Create" uses the "notionApi" credential "Integration"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/pages" had a JSON body matching:
      """
      {
        "parent": {"page_id": "parent1"},
        "properties": {"title": [{"text": {"content": "New page"}}]},
        "icon": {"emoji": "📄"}
      }
      """

  Scenario: Getting a page by URL extracts the trailing ID
    Given the mock service responds to GET "/v1/pages/b4eeb113e118403aa450af65ac25f0b9" with status 200 and body:
      """
      {"object": "page", "id": "b4eeb113e118403aa450af65ac25f0b9", "parent": {"type": "page_id"}, "properties": {"title": {"title": [{"plain_text": "My Page"}]}}, "url": "https://notion.so/My-Page-b4eeb113e118403aa450af65ac25f0b9"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "page", "operation": "get", "pageId": {"mode": "url", "value": "https://www.notion.so/My-Page-b4eeb113e118403aa450af65ac25f0b9"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": "b4eeb113e118403aa450af65ac25f0b9", "name": "My Page", "url": "https://notion.so/My-Page-b4eeb113e118403aa450af65ac25f0b9"}]
      """

  Scenario: Searching pages sends the object filter and truncates to the limit without requesting extra pages
    Given the mock service responds to POST "/v1/search" with status 200 and body:
      """
      {
        "results": [
          {"object": "page", "id": "p1", "parent": {"type": "workspace"}, "properties": {"title": {"title": [{"plain_text": "A"}]}}},
          {"object": "page", "id": "p2", "parent": {"type": "workspace"}, "properties": {"title": {"title": [{"plain_text": "B"}]}}},
          {"object": "page", "id": "p3", "parent": {"type": "workspace"}, "properties": {"title": {"title": [{"plain_text": "C"}]}}}
        ],
        "has_more": false
      }
      """
    And a workflow with nodes:
      | name   | type   |
      | Start  | manualTrigger |
      | Search | notion |
    And the node "Search" has parameters:
      """
      {
        "resource": "page",
        "operation": "search",
        "text": "",
        "returnAll": false,
        "limit": 2,
        "simple": true,
        "options": {"filter": {"filters": {"property": "object", "value": "page"}}}
      }
      """
    And the node "Search" uses the "notionApi" credential "Integration"
    And the connections "Start -> Search"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/v1/search"
    And the last request to "/v1/search" had a JSON body matching:
      """
      {"filter": {"property": "object", "value": "page"}}
      """
    And the node "Search" outputs 2 items

  # ---- user ------------------------------------------------------------

  Scenario: Getting a user returns the raw response
    Given the mock service responds to GET "/v1/users/user1" with status 200 and body:
      """
      {"object": "user", "id": "user1", "name": "Ada Lovelace"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "user", "operation": "get", "userId": "user1"}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"object": "user", "id": "user1", "name": "Ada Lovelace"}]
      """

  Scenario: Getting many users fetches every page before truncating to the limit
    Given the mock service responds to GET "/v1/users" with status 200 and body:
      """
      {"results": [{"id": "u1"}, {"id": "u2"}, {"id": "u3"}], "has_more": false}
      """
    And a workflow with nodes:
      | name    | type   |
      | Start   | manualTrigger |
      | GetMany | notion |
    And the node "GetMany" has parameters:
      """
      {"resource": "user", "operation": "getAll", "returnAll": false, "limit": 2}
      """
    And the node "GetMany" uses the "notionApi" credential "Integration"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/v1/users"
    And the node "GetMany" outputs:
      """
      [{"id": "u1"}, {"id": "u2"}]
      """

  # ---- errors, auth, continueOnFail, and unsupported operations ------------

  Scenario: A 400 from Notion maps to n8n's bad-request message
    Given the mock service responds to GET "/v1/pages/bad" with status 400 and body:
      """
      {"object": "error", "status": 400, "code": "validation_error", "message": "path failed validation"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "bad"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Bad request - please check your parameters"

  Scenario: A 401 from Notion maps to n8n's authorization-failed message
    Given the mock service responds to GET "/v1/pages/page1" with status 401 and body:
      """
      {"object": "error", "status": 401, "code": "unauthorized", "message": "API token is invalid"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 404 from Notion maps to n8n's resource-not-found message
    Given the mock service responds to GET "/v1/pages/missing" with status 404 and body:
      """
      {"object": "error", "status": 404, "code": "object_not_found", "message": "Could not find page"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "missing"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The resource you are requesting could not be found"

  Scenario: A 429 from Notion maps to n8n's rate-limit message
    Given the mock service responds to GET "/v1/pages/page1" with status 429 and body:
      """
      {"object": "error", "status": 429, "code": "rate_limited", "message": "You have been rate limited"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The service is receiving too many requests from you"

  Scenario: A missing Notion credential fails with a clear message
    Given a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: The API key never appears in the execution data
    Given the mock service responds to GET "/v1/pages/page1" with status 401 and body:
      """
      {"object": "error", "status": 401, "message": "API token is invalid"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "page1"}, "simple": true}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "secret_test_token"

  Scenario: continueOnFail turns a Notion error into an error item
    Given the mock service responds to GET "/v1/pages/missing" with status 404 and body:
      """
      {"object": "error", "status": 404, "message": "Could not find page"}
      """
    And a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"resource": "databasePage", "operation": "get", "pageId": {"mode": "id", "value": "missing"}, "simple": true}
      """
    And the node "Get" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Get" uses the "notionApi" credential "Integration"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"error": "$contains:The resource you are requesting could not be found"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Comm | notion |
    And the node "Comm" has parameters:
      """
      {"resource": "comment", "operation": "create"}
      """
    And the node "Comm" uses the "notionApi" credential "Integration"
    And the connections "Start -> Comm"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | U    | notion |
    And the node "U" has parameters:
      """
      {"resource": "user", "operation": "delete"}
      """
    And the node "U" uses the "notionApi" credential "Integration"
    And the connections "Start -> U"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: OAuth2 authentication is not supported natively yet
    Given a workflow with nodes:
      | name | type   |
      | Start| manualTrigger |
      | Get  | notion |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "user", "operation": "get", "userId": "u1"}
      """
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
