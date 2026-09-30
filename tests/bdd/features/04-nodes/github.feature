@spec-6.6 @phase-4 @node-github
Feature: GitHub node
  Consumes the GitHub REST API (v1/1.1, as the n8n 2.35.7 editor creates the
  node) against the `githubApi` (access token) and `githubOAuth2Api`
  credentials. `pullRequest`, `workflow` and `organization:getMembers` are
  out of scope; anything unimplemented fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "PAT" of type "githubApi" with the data:
      """
      {"accessToken": "ghp_test_token", "server": "%{MOCK_URL}"}
      """

  # ---- file ------------------------------------------------------------

  Scenario: Creating a file base64-encodes plain-text content
    Given the mock service responds to PUT "/repos/octocat/hello-world/contents/docs%2FREADME.md" with status 201 and body:
      """
      {"content": {"path": "docs/README.md", "sha": "abc123"}, "commit": {"sha": "def456"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {
        "resource": "file",
        "operation": "create",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "docs/README.md",
        "binaryData": false,
        "fileContent": "hello world",
        "commitMessage": "add readme",
        "additionalParameters": {}
      }
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/contents/docs%2FREADME.md" had the header "authorization" equal to "token ghp_test_token"
    And the last request to "/repos/octocat/hello-world/contents/docs%2FREADME.md" had a JSON body matching:
      """
      {"message": "add readme", "content": "aGVsbG8gd29ybGQ="}
      """
    And the node "Create" outputs:
      """
      [{"content": {"path": "docs/README.md", "sha": "abc123"}, "commit": {"sha": "def456"}}]
      """

  Scenario: Creating a file with already-base64 content passes it through unchanged
    Given the mock service responds to PUT "/repos/octocat/hello-world/contents/data.bin" with status 201 and body:
      """
      {"content": {"path": "data.bin"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {
        "resource": "file",
        "operation": "create",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "data.bin",
        "binaryData": false,
        "fileContent": "aGVsbG8=",
        "commitMessage": "add data",
        "additionalParameters": {}
      }
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/contents/data.bin" had a JSON body matching:
      """
      {"content": "aGVsbG8="}
      """

  Scenario: Creating a file from binary input data, with author/committer/branch
    Given the mock service responds to PUT "/repos/octocat/hello-world/contents/image.png" with status 201 and body:
      """
      {"content": {"path": "image.png"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {
        "resource": "file",
        "operation": "create",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "image.png",
        "binaryData": true,
        "binaryPropertyName": "data",
        "commitMessage": "add image",
        "additionalParameters": {
          "author": {"name": "Ada", "email": "ada@example.com"},
          "committer": {"name": "Bot", "email": "bot@example.com"},
          "branch": {"branch": "feature-x"}
        }
      }
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "PNGDATA" and mime type "image/png"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/contents/image.png" had a JSON body matching:
      """
      {
        "message": "add image",
        "content": "UE5HREFUQQ==",
        "branch": "feature-x",
        "author": {"name": "Ada", "email": "ada@example.com"},
        "committer": {"name": "Bot", "email": "bot@example.com"}
      }
      """

  Scenario: Editing a file fetches its current SHA first, unencoded, then PUTs the change
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/docs/README.md" with status 200 and body:
      """
      {"sha": "oldsha123"}
      """
    And the mock service responds to PUT "/repos/octocat/hello-world/contents/docs%2FREADME.md" with status 200 and body:
      """
      {"content": {"path": "docs/README.md", "sha": "newsha456"}}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Edit | github        |
    And the node "Edit" has parameters:
      """
      {
        "resource": "file",
        "operation": "edit",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "docs/README.md",
        "binaryData": false,
        "fileContent": "new content",
        "commitMessage": "update readme",
        "additionalParameters": {}
      }
      """
    And the node "Edit" uses the "githubApi" credential "PAT"
    And the connections "Start -> Edit"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/repos/octocat/hello-world/contents/docs/README.md"
    And the last request to "/repos/octocat/hello-world/contents/docs%2FREADME.md" had a JSON body matching:
      """
      {"sha": "oldsha123", "message": "update readme", "content": "bmV3IGNvbnRlbnQ="}
      """

  Scenario: Deleting a file fetches its SHA first, then DELETEs with it
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/old.txt" with status 200 and body:
      """
      {"sha": "shatodelete"}
      """
    And the mock service responds to DELETE "/repos/octocat/hello-world/contents/old.txt" with status 200 and body:
      """
      {"commit": {"sha": "aftersha"}}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | github        |
    And the node "Delete" has parameters:
      """
      {
        "resource": "file",
        "operation": "delete",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "old.txt",
        "commitMessage": "remove old file",
        "additionalParameters": {}
      }
      """
    And the node "Delete" uses the "githubApi" credential "PAT"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/contents/old.txt" had a JSON body matching:
      """
      {"sha": "shatodelete", "message": "remove old file"}
      """
    And the node "Delete" outputs:
      """
      [{"commit": {"sha": "aftersha"}}]
      """

  Scenario: Getting a file as binary decodes its base64 content
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/docs%2FREADME.md" with status 200 and body:
      """
      {"content": "aGVsbG8gd29ybGQ=\n", "encoding": "base64", "path": "docs/README.md", "sha": "abc"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {
        "resource": "file",
        "operation": "get",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "docs/README.md",
        "asBinaryProperty": true,
        "binaryPropertyName": "data",
        "additionalParameters": {}
      }
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Get": [
                {"data": {"main": [[{"binary": {"data": {"data": "aGVsbG8gd29ybGQ=", "fileName": "README.md", "mimeType": "text/plain"}}}]]}}
              ]
            }
          }
        }
      }
      """

  Scenario: Getting a file as raw JSON (not binary) returns the API response
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/docs%2FREADME.md" with status 200 and body:
      """
      {"content": "aGVsbG8=", "path": "docs/README.md", "sha": "abc"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {
        "resource": "file",
        "operation": "get",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "docs/README.md",
        "asBinaryProperty": false,
        "additionalParameters": {}
      }
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"content": "aGVsbG8=", "path": "docs/README.md", "sha": "abc"}]
      """

  Scenario: Getting a file whose path is actually a folder fails with a clear error
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/docs" with status 200 and body:
      """
      [{"name": "a.md", "path": "docs/a.md"}, {"name": "b.md", "path": "docs/b.md"}]
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {
        "resource": "file",
        "operation": "get",
        "owner": "octocat",
        "repository": "hello-world",
        "filePath": "docs",
        "asBinaryProperty": true,
        "binaryPropertyName": "data",
        "additionalParameters": {}
      }
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "File Path is a folder, not a file."

  Scenario: Listing a folder's contents outputs one item per entry
    Given the mock service responds to GET "/repos/octocat/hello-world/contents/docs" with status 200 and body:
      """
      [{"name": "a.md", "type": "file"}, {"name": "b.md", "type": "file"}]
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | List | github        |
    And the node "List" has parameters:
      """
      {"resource": "file", "operation": "list", "owner": "octocat", "repository": "hello-world", "filePath": "docs"}
      """
    And the node "List" uses the "githubApi" credential "PAT"
    And the connections "Start -> List"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs:
      """
      [{"name": "a.md", "type": "file"}, {"name": "b.md", "type": "file"}]
      """

  # ---- issue -------------------------------------------------------------

  Scenario: Creating an issue maps label/assignee collections to string arrays
    Given the mock service responds to POST "/repos/octocat/hello-world/issues" with status 201 and body:
      """
      {"number": 42, "title": "Bug found", "state": "open"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {
        "resource": "issue",
        "operation": "create",
        "owner": "octocat",
        "repository": "hello-world",
        "title": "Bug found",
        "body": "Steps to reproduce...",
        "labels": [{"label": "bug"}, {"label": "p1"}],
        "assignees": [{"assignee": "octocat"}]
      }
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/issues" had a JSON body matching:
      """
      {"title": "Bug found", "body": "Steps to reproduce...", "labels": ["bug", "p1"], "assignees": ["octocat"]}
      """
    And the node "Create" outputs:
      """
      [{"number": 42, "title": "Bug found", "state": "open"}]
      """

  Scenario: Creating a comment on an issue
    Given the mock service responds to POST "/repos/octocat/hello-world/issues/42/comments" with status 201 and body:
      """
      {"id": 999, "body": "Looking into it"}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Comment | github        |
    And the node "Comment" has parameters:
      """
      {"resource": "issue", "operation": "createComment", "owner": "octocat", "repository": "hello-world", "issueNumber": 42, "body": "Looking into it"}
      """
    And the node "Comment" uses the "githubApi" credential "PAT"
    And the connections "Start -> Comment"
    When I execute the workflow
    Then the execution succeeds
    And the node "Comment" outputs:
      """
      [{"id": 999, "body": "Looking into it"}]
      """

  Scenario: Editing an issue maps editFields labels/assignees and passes scalar fields through
    Given the mock service responds to PATCH "/repos/octocat/hello-world/issues/42" with status 200 and body:
      """
      {"number": 42, "state": "closed"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Edit | github        |
    And the node "Edit" has parameters:
      """
      {
        "resource": "issue",
        "operation": "edit",
        "owner": "octocat",
        "repository": "hello-world",
        "issueNumber": 42,
        "editFields": {
          "state": "closed",
          "state_reason": "completed",
          "labels": [{"label": "done"}],
          "assignees": [{"assignee": "octocat"}]
        }
      }
      """
    And the node "Edit" uses the "githubApi" credential "PAT"
    And the connections "Start -> Edit"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/issues/42" had a JSON body matching:
      """
      {"state": "closed", "state_reason": "completed", "labels": ["done"], "assignees": ["octocat"]}
      """

  Scenario: Getting an issue
    Given the mock service responds to GET "/repos/octocat/hello-world/issues/42" with status 200 and body:
      """
      {"number": 42, "title": "Bug found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "issue", "operation": "get", "owner": "octocat", "repository": "hello-world", "issueNumber": 42}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"number": 42, "title": "Bug found"}]
      """

  Scenario: Locking an issue passes the lock reason as a query parameter and leaves output unchanged
    Given the mock service responds to PUT "/repos/octocat/hello-world/issues/42/lock" with status 204
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Lock | github        |
    And the node "Lock" has parameters:
      """
      {"resource": "issue", "operation": "lock", "owner": "octocat", "repository": "hello-world", "issueNumber": 42, "lockReason": "spam"}
      """
    And the node "Lock" uses the "githubApi" credential "PAT"
    And the connections "Start -> Lock"
    And the trigger outputs the items:
      """
      [{"marker": "unchanged"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/issues/42/lock" had the query parameter "lock_reason" equal to "spam"
    And the node "Lock" outputs:
      """
      [{"marker": "unchanged"}]
      """

  # ---- release -------------------------------------------------------------

  Scenario: Creating a release merges additionalFields with the tag name
    Given the mock service responds to POST "/repos/octocat/hello-world/releases" with status 201 and body:
      """
      {"id": 1, "tag_name": "v1.0.0"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {
        "resource": "release",
        "operation": "create",
        "owner": "octocat",
        "repository": "hello-world",
        "releaseTag": "v1.0.0",
        "additionalFields": {"name": "First release", "draft": false, "prerelease": false}
      }
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/releases" had a JSON body matching:
      """
      {"tag_name": "v1.0.0", "name": "First release", "draft": false, "prerelease": false}
      """
    And the node "Create" outputs:
      """
      [{"id": 1, "tag_name": "v1.0.0"}]
      """

  Scenario: Deleting a release always outputs {"success": true}
    Given the mock service responds to DELETE "/repos/octocat/hello-world/releases/1" with status 204
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | github        |
    And the node "Delete" has parameters:
      """
      {"resource": "release", "operation": "delete", "owner": "octocat", "repository": "hello-world", "release_id": "1"}
      """
    And the node "Delete" uses the "githubApi" credential "PAT"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs:
      """
      [{"success": true}]
      """

  Scenario: Getting a release
    Given the mock service responds to GET "/repos/octocat/hello-world/releases/1" with status 200 and body:
      """
      {"id": 1, "tag_name": "v1.0.0"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "release", "operation": "get", "owner": "octocat", "repository": "hello-world", "release_id": "1"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": 1, "tag_name": "v1.0.0"}]
      """

  Scenario: Getting a limited number of releases sends per_page from the Limit field
    Given the mock service responds to GET "/repos/octocat/hello-world/releases" with status 200 and body:
      """
      [{"id": 1}, {"id": 2}]
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | GetMany | github        |
    And the node "GetMany" has parameters:
      """
      {"resource": "release", "operation": "getAll", "owner": "octocat", "repository": "hello-world", "returnAll": false, "limit": 5}
      """
    And the node "GetMany" uses the "githubApi" credential "PAT"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/releases" had the query parameter "per_page" equal to "5"
    And the node "GetMany" outputs:
      """
      [{"id": 1}, {"id": 2}]
      """

  Scenario: Getting all releases paginates while the Link response header says there is a next page
    Given the mock service responds to GET "/repos/octocat/hello-world/releases" with query parameter "page" equal to "1" with status 200, header "Link" "<%{MOCK_URL}/repos/octocat/hello-world/releases?page=2>; rel=\"next\"" and body:
      """
      [{"id": 1}]
      """
    And the mock service responds to GET "/repos/octocat/hello-world/releases" with query parameter "page" equal to "2" with status 200 and body:
      """
      [{"id": 2}]
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | GetAll  | github        |
    And the node "GetAll" has parameters:
      """
      {"resource": "release", "operation": "getAll", "owner": "octocat", "repository": "hello-world", "returnAll": true}
      """
    And the node "GetAll" uses the "githubApi" credential "PAT"
    And the connections "Start -> GetAll"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/repos/octocat/hello-world/releases"
    And the node "GetAll" outputs:
      """
      [{"id": 1}, {"id": 2}]
      """

  Scenario: Updating a release sends additionalFields as-is
    Given the mock service responds to PATCH "/repos/octocat/hello-world/releases/1" with status 200 and body:
      """
      {"id": 1, "name": "Renamed"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Update| github        |
    And the node "Update" has parameters:
      """
      {"resource": "release", "operation": "update", "owner": "octocat", "repository": "hello-world", "release_id": "1", "additionalFields": {"name": "Renamed"}}
      """
    And the node "Update" uses the "githubApi" credential "PAT"
    And the connections "Start -> Update"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/releases/1" had a JSON body matching:
      """
      {"name": "Renamed"}
      """

  # ---- repository -------------------------------------------------------------

  Scenario: Getting a repository
    Given the mock service responds to GET "/repos/octocat/hello-world" with status 200 and body:
      """
      {"id": 1, "full_name": "octocat/hello-world"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": 1, "full_name": "octocat/hello-world"}]
      """

  Scenario: Getting a repository's license
    Given the mock service responds to GET "/repos/octocat/hello-world/license" with status 200 and body:
      """
      {"license": {"key": "mit"}}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | License | github        |
    And the node "License" has parameters:
      """
      {"resource": "repository", "operation": "getLicense", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "License" uses the "githubApi" credential "PAT"
    And the connections "Start -> License"
    When I execute the workflow
    Then the execution succeeds
    And the node "License" outputs:
      """
      [{"license": {"key": "mit"}}]
      """

  Scenario: Getting a repository's community profile
    Given the mock service responds to GET "/repos/octocat/hello-world/community/profile" with status 200 and body:
      """
      {"health_percentage": 80}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Profile | github        |
    And the node "Profile" has parameters:
      """
      {"resource": "repository", "operation": "getProfile", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Profile" uses the "githubApi" credential "PAT"
    And the connections "Start -> Profile"
    When I execute the workflow
    Then the execution succeeds
    And the node "Profile" outputs:
      """
      [{"health_percentage": 80}]
      """

  Scenario: Getting a repository's issues applies filters and the limit
    Given the mock service responds to GET "/repos/octocat/hello-world/issues" with status 200 and body:
      """
      [{"number": 1}, {"number": 2}]
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Issues| github        |
    And the node "Issues" has parameters:
      """
      {
        "resource": "repository",
        "operation": "getIssues",
        "owner": "octocat",
        "repository": "hello-world",
        "returnAll": false,
        "limit": 10,
        "getRepositoryIssuesFilters": {"state": "open", "sort": "updated"}
      }
      """
    And the node "Issues" uses the "githubApi" credential "PAT"
    And the connections "Start -> Issues"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/issues" had the query parameter "state" equal to "open"
    And the last request to "/repos/octocat/hello-world/issues" had the query parameter "sort" equal to "updated"
    And the last request to "/repos/octocat/hello-world/issues" had the query parameter "per_page" equal to "10"
    And the node "Issues" outputs:
      """
      [{"number": 1}, {"number": 2}]
      """

  Scenario: Getting a repository's pull requests applies filters
    Given the mock service responds to GET "/repos/octocat/hello-world/pulls" with status 200 and body:
      """
      [{"number": 5}]
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Pulls| github        |
    And the node "Pulls" has parameters:
      """
      {
        "resource": "repository",
        "operation": "getPullRequests",
        "owner": "octocat",
        "repository": "hello-world",
        "returnAll": false,
        "limit": 20,
        "getRepositoryPullRequestsFilters": {"state": "all"}
      }
      """
    And the node "Pulls" uses the "githubApi" credential "PAT"
    And the connections "Start -> Pulls"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/pulls" had the query parameter "state" equal to "all"
    And the node "Pulls" outputs:
      """
      [{"number": 5}]
      """

  Scenario: Listing a repository's popular content paths
    Given the mock service responds to GET "/repos/octocat/hello-world/traffic/popular/paths" with status 200 and body:
      """
      [{"path": "/", "count": 100}]
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Paths | github        |
    And the node "Paths" has parameters:
      """
      {"resource": "repository", "operation": "listPopularPaths", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Paths" uses the "githubApi" credential "PAT"
    And the connections "Start -> Paths"
    When I execute the workflow
    Then the execution succeeds
    And the node "Paths" outputs:
      """
      [{"path": "/", "count": 100}]
      """

  Scenario: Listing a repository's top referrers
    Given the mock service responds to GET "/repos/octocat/hello-world/traffic/popular/referrers" with status 200 and body:
      """
      [{"referrer": "google.com", "count": 50}]
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | Referrers | github        |
    And the node "Referrers" has parameters:
      """
      {"resource": "repository", "operation": "listReferrers", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Referrers" uses the "githubApi" credential "PAT"
    And the connections "Start -> Referrers"
    When I execute the workflow
    Then the execution succeeds
    And the node "Referrers" outputs:
      """
      [{"referrer": "google.com", "count": 50}]
      """

  # ---- review -------------------------------------------------------------

  Scenario: Creating an approving review
    Given the mock service responds to POST "/repos/octocat/hello-world/pulls/7/reviews" with status 200 and body:
      """
      {"id": 100, "state": "APPROVED"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Review | github        |
    And the node "Review" has parameters:
      """
      {"resource": "review", "operation": "create", "owner": "octocat", "repository": "hello-world", "pullRequestNumber": 7, "event": "approve", "additionalFields": {}}
      """
    And the node "Review" uses the "githubApi" credential "PAT"
    And the connections "Start -> Review"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/pulls/7/reviews" had a JSON body matching:
      """
      {"event": "APPROVE"}
      """
    And the node "Review" outputs:
      """
      [{"id": 100, "state": "APPROVED"}]
      """

  Scenario: Requesting changes on a review includes the body and snake_cases the event
    Given the mock service responds to POST "/repos/octocat/hello-world/pulls/7/reviews" with status 200 and body:
      """
      {"id": 101, "state": "CHANGES_REQUESTED"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Review | github        |
    And the node "Review" has parameters:
      """
      {"resource": "review", "operation": "create", "owner": "octocat", "repository": "hello-world", "pullRequestNumber": 7, "event": "requestChanges", "body": "Please fix the tests", "additionalFields": {}}
      """
    And the node "Review" uses the "githubApi" credential "PAT"
    And the connections "Start -> Review"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/pulls/7/reviews" had a JSON body matching:
      """
      {"event": "REQUEST_CHANGES", "body": "Please fix the tests"}
      """

  Scenario: Getting a review
    Given the mock service responds to GET "/repos/octocat/hello-world/pulls/7/reviews/100" with status 200 and body:
      """
      {"id": 100, "state": "APPROVED"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "review", "operation": "get", "owner": "octocat", "repository": "hello-world", "pullRequestNumber": 7, "reviewId": "100"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"id": 100, "state": "APPROVED"}]
      """

  Scenario: Getting many reviews for a pull request
    Given the mock service responds to GET "/repos/octocat/hello-world/pulls/7/reviews" with status 200 and body:
      """
      [{"id": 100}, {"id": 101}]
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | GetMany | github        |
    And the node "GetMany" has parameters:
      """
      {"resource": "review", "operation": "getAll", "owner": "octocat", "repository": "hello-world", "pullRequestNumber": 7, "returnAll": false, "limit": 30}
      """
    And the node "GetMany" uses the "githubApi" credential "PAT"
    And the connections "Start -> GetMany"
    When I execute the workflow
    Then the execution succeeds
    And the node "GetMany" outputs:
      """
      [{"id": 100}, {"id": 101}]
      """

  Scenario: Updating a review's body
    Given the mock service responds to PUT "/repos/octocat/hello-world/pulls/7/reviews/100" with status 200 and body:
      """
      {"id": 100, "body": "Updated"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Update| github        |
    And the node "Update" has parameters:
      """
      {"resource": "review", "operation": "update", "owner": "octocat", "repository": "hello-world", "pullRequestNumber": 7, "reviewId": "100", "body": "Updated"}
      """
    And the node "Update" uses the "githubApi" credential "PAT"
    And the connections "Start -> Update"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world/pulls/7/reviews/100" had a JSON body matching:
      """
      {"body": "Updated"}
      """

  # ---- user / organization -------------------------------------------------------------

  Scenario: Getting a user's repositories
    Given the mock service responds to GET "/users/octocat/repos" with status 200 and body:
      """
      [{"name": "hello-world"}]
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Repos | github        |
    And the node "Repos" has parameters:
      """
      {"resource": "user", "operation": "getRepositories", "owner": "octocat", "returnAll": false, "limit": 25}
      """
    And the node "Repos" uses the "githubApi" credential "PAT"
    And the connections "Start -> Repos"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/users/octocat/repos" had the query parameter "per_page" equal to "25"
    And the node "Repos" outputs:
      """
      [{"name": "hello-world"}]
      """

  Scenario: Getting a user's assigned issues hits the fixed /issues endpoint
    Given the mock service responds to GET "/issues" with status 200 and body:
      """
      [{"number": 9, "title": "assigned to me"}]
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Issues | github        |
    And the node "Issues" has parameters:
      """
      {"resource": "user", "operation": "getUserIssues", "returnAll": false, "limit": 25, "getUserIssuesFilters": {"state": "open"}}
      """
    And the node "Issues" uses the "githubApi" credential "PAT"
    And the connections "Start -> Issues"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/issues" had the query parameter "state" equal to "open"
    And the node "Issues" outputs:
      """
      [{"number": 9, "title": "assigned to me"}]
      """

  Scenario: Inviting a user to an organization
    Given the mock service responds to POST "/orgs/my-org/invitations" with status 201 and body:
      """
      {"id": 1, "email": "new@example.com"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Invite | github        |
    And the node "Invite" has parameters:
      """
      {"resource": "user", "operation": "invite", "organization": "my-org", "email": "new@example.com"}
      """
    And the node "Invite" uses the "githubApi" credential "PAT"
    And the connections "Start -> Invite"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/orgs/my-org/invitations" had a JSON body matching:
      """
      {"email": "new@example.com"}
      """
    And the node "Invite" outputs:
      """
      [{"id": 1, "email": "new@example.com"}]
      """

  Scenario: Getting an organization's repositories
    Given the mock service responds to GET "/orgs/my-org/repos" with status 200 and body:
      """
      [{"name": "repo-a"}, {"name": "repo-b"}]
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Repos | github        |
    And the node "Repos" has parameters:
      """
      {"resource": "organization", "operation": "getRepositories", "owner": "my-org", "returnAll": false, "limit": 50}
      """
    And the node "Repos" uses the "githubApi" credential "PAT"
    And the connections "Start -> Repos"
    When I execute the workflow
    Then the execution succeeds
    And the node "Repos" outputs:
      """
      [{"name": "repo-a"}, {"name": "repo-b"}]
      """

  # ---- errors, auth, continueOnFail, and unsupported operations ------------

  Scenario: A 401 from GitHub maps to n8n's authorization-failed message
    Given the mock service responds to GET "/repos/octocat/hello-world" with status 401 and body:
      """
      {"message": "Bad credentials"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 404 from GitHub maps to n8n's resource-not-found message
    Given the mock service responds to GET "/repos/octocat/missing" with status 404 and body:
      """
      {"message": "Not Found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "missing"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "The resource you are requesting could not be found"

  Scenario: A 422 from GitHub maps to n8n's generic 4XX message
    Given the mock service responds to POST "/repos/octocat/hello-world/issues" with status 422 and body:
      """
      {"message": "Validation Failed", "errors": [{"field": "title", "code": "missing_field"}]}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Create | github        |
    And the node "Create" has parameters:
      """
      {"resource": "issue", "operation": "create", "owner": "octocat", "repository": "hello-world", "title": "", "body": "", "labels": [], "assignees": []}
      """
    And the node "Create" uses the "githubApi" credential "PAT"
    And the connections "Start -> Create"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Your request is invalid or could not be processed by the service"

  Scenario: A missing GitHub credential fails with a clear message
    Given a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: The access token never appears in the execution data
    Given the mock service responds to GET "/repos/octocat/hello-world" with status 401 and body:
      """
      {"message": "Bad credentials"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "ghp_test_token"

  Scenario: continueOnFail turns a GitHub error into an error item for a data-returning operation
    Given the mock service responds to GET "/repos/octocat/missing" with status 404 and body:
      """
      {"message": "Not Found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"resource": "repository", "operation": "get", "owner": "octocat", "repository": "missing"}
      """
    And the node "Get" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Get" uses the "githubApi" credential "PAT"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the node "Get" outputs:
      """
      [{"error": "$contains:The resource you are requesting could not be found"}]
      """

  Scenario: continueOnFail on a pass-through operation replaces the item's json with the error
    Given the mock service responds to PUT "/repos/octocat/hello-world/issues/42/lock" with status 404 and body:
      """
      {"message": "Not Found"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Lock | github        |
    And the node "Lock" has parameters:
      """
      {"resource": "issue", "operation": "lock", "owner": "octocat", "repository": "hello-world", "issueNumber": 42, "lockReason": "spam"}
      """
    And the node "Lock" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Lock" uses the "githubApi" credential "PAT"
    And the connections "Start -> Lock"
    And the trigger outputs the items:
      """
      [{"marker": "unchanged"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Lock" outputs:
      """
      [{"error": "$contains:The resource you are requesting could not be found"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | PR   | github        |
    And the node "PR" has parameters:
      """
      {"resource": "pullRequest", "operation": "create", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "PR" uses the "githubApi" credential "PAT"
    And the connections "Start -> PR"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Members | github        |
    And the node "Members" has parameters:
      """
      {"resource": "organization", "operation": "getMembers", "owner": "octocat"}
      """
    And the node "Members" uses the "githubApi" credential "PAT"
    And the connections "Start -> Members"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: Posting with OAuth2 authentication sends a Bearer token
    Given the credential "OAuth App" of type "githubOAuth2Api" with the data:
      """
      {"server": "%{MOCK_URL}", "oauthTokenData": {"access_token": "gho_oauth_token"}}
      """
    And the mock service responds to GET "/repos/octocat/hello-world" with status 200 and body:
      """
      {"id": 1, "full_name": "octocat/hello-world"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Get" uses the "githubOAuth2Api" credential "OAuth App"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/repos/octocat/hello-world" had the header "authorization" equal to "Bearer gho_oauth_token"

  Scenario: A GitHub OAuth2 credential that was never connected fails with a clear message
    Given the credential "Unconnected" of type "githubOAuth2Api" with the data:
      """
      {"server": "%{MOCK_URL}"}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Get  | github        |
    And the node "Get" has parameters:
      """
      {"authentication": "oAuth2", "resource": "repository", "operation": "get", "owner": "octocat", "repository": "hello-world"}
      """
    And the node "Get" uses the "githubOAuth2Api" credential "Unconnected"
    And the connections "Start -> Get"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not connected"
