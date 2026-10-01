@spec-6.6 @phase-4 @node-google-drive
Feature: Google Drive node
  Consumes the Google Drive v3 API (node v3, as the n8n 2.35.7 editor
  creates it) against the `googleDriveOAuth2Api` and `googleApi`
  (service-account JWT) credentials. Implements the `file` resource's
  copy, createFromText, deleteFile, download, move, share, update and
  upload operations; the `fileFolder` resource's search; the `folder`
  resource's create, deleteFolder and share; and the `drive` resource's
  create, deleteDrive, get, list and update (shared drives). Anything else
  fails with a clear message.

  Background:
    Given a mock HTTP service
    And the credential "Service Account" of type "googleApi" with the data:
      """
      {
        "email": "svc@example.iam.gserviceaccount.com",
        "privateKey": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCgHQNrUetQxEOs\nNfsDt8iMM4/YRAkvAXJQNN7J03CZrB5qg/4NE9yhLA/Iy+VMX+5pBrxoXn5ScTMD\ns0g1qtRR5D2tOLsTjKC+m7sOvVzRh4vJWAXAmaWxfrCg59M1AkYvbIDHs3D5hnTq\nG2ANmA5C9l5zYCeQRH3Niama6ZcI+aaOlwGWklVfTCOBlbIjhqGTr1XYo+vEu+d7\nCZRUB1AR8AphEUQKKxVzQCftnKQAt6lIYWj0Ivt57JtYeKtmvyM1a/nIm/7VH0UH\nctZQljtVKShYF/Mx0bgugJRzaXwFdoKHHddP+eZOUlxmrYdNM4LmU+KofnbMwUpN\nXnRq7OWlAgMBAAECggEAI7NMaZ/GqQfV8f5IRaQzKIWvr2A+Lvey2izrtxM44l7l\nGklrelWBJoO5UkOaUTz7nvnkRR46qt24Kv1M3sCEHm1WjjrdxaZfyhhVrVvuJ/8B\n3WdNwwvPTOQLdJk0N/fVl+nozf5V9KE1DOUFNgq/NVGTLkUUlT+cUFm/Uj3+0f5k\nCFcD/K8SnaukzCRXhXD82j+/6dtAaPyjkEHNKHYofwzvQPtYTdpwYS0Dts9hXmyb\n3g7DU2ewyPevWZSoZ4n9wKTpQ6GamGTdJEtu40VIJGIxQ/yBQYz7LXm1X/EriDMJ\nex4hoTC18Gvtjyo7uFA+nINcwqpG/V0W7D3+CNSdaQKBgQDNVXwE7rn9DOhlzHMm\nFkcHe7p3tgcCts/NC775Kyw+So9+qFRQRdQ1Hminx5w4a3+fR6eVGMf57Xl2AkGy\nat8F6KXn3iXT4qml/J/arkgGsd9YrUBAtN173owdEoNJP1wE4ho7Q58Y+U+zQ51D\n/SZYpRhjiAK+1eJpJWJRfXzGiQKBgQDHnwnfu5Vh4Of1BMTuhVK6O3lxolYYF8w3\nx+H65EN2EjXlG/EYw3RsG9QHhTqF6JVM6vyqFZjRfcuz9DET5fcn0DKvR3D7iVeU\ne/2pCHE+uvEZ9Su0gHZiXT68c/1c+9C7nXqexW16uTm5G7WOZMpEEaAIdRp/toEu\n5AWKDS8fPQKBgDxYvFtCwhyx93c7sDfoYjW70mCueb79dXMg2Z6nZphkF2o1FJqG\n+0glSMLOsoYOafKo/4KdRuCYP5NENIS4ThWRe3j63Ak623syFNUTVY3KJwcL3A9o\nWJO4I1vD/hu/6E5zGRyD0jVnyFm6LHU36FYzJ0jRR2VIvQMD/rJOfCZpAoGBAKnf\nIu3rmXGTjJCrIFLBzeaBGhWjSZRzG+wUArAYc3gUgxyWrJKgMYCWJdbIf0bY58Ru\n69hpTIRpgmF+2gzO04Zj293g87p547eN1Ax2DLiPKQEn66tM7nFCXFLOebsY50Xg\n+yoFY+bdnMtzUwr7pkxKm17XGFe6HTCkBjq2gXUZAoGBAJ8KzbfnHGmmpimolnL+\nTPXNJP6QSwQi92UgIIbh8/BsFyW+5X/3cHhmFo5IlHlOktbT+zru6jv3EG6omqNQ\nWRlFVR6p6VwWjqzTZbR+dZUbRpSAk3jFZ+HIIkhfYpupMYySUf6UpQL10hQWrlYT\n87vTIKToXQtdQtTCFKSYkryf\n-----END PRIVATE KEY-----",
        "url": "%{MOCK_URL}",
        "tokenUrl": "%{MOCK_URL}/oauth2/token"
      }
      """
    And the mock service responds to POST "/oauth2/token" with status 200 and body:
      """
      {"access_token": "svc-token", "expires_in": 3600}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Node  | googleDrive   |
    And the connections "Start -> Node"
    And the node "Node" uses the "googleApi" credential "Service Account"

  # ---- file: copy -----------------------------------------------------------

  Scenario: Copy a file, deriving the name from the original when none is given
    Given the mock service responds to GET "/drive/v3/files/F1" with status 200 and body:
      """
      {"name": "Original.txt"}
      """
    And the mock service responds to POST "/drive/v3/files/F1/copy" with status 200 and body:
      """
      {"id": "F2", "name": "Copy of Original.txt"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "copy", "fileId": {"mode": "id", "value": "F1"}, "name": "", "sameFolder": true, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1/copy" had a JSON body matching:
      """
      {"copyRequiresWriterPermission": false, "name": "Copy of Original.txt"}
      """
    And the node "Node" outputs:
      """
      [{"id": "F2", "name": "Copy of Original.txt"}]
      """

  Scenario: Copy a file into a different folder with an explicit name
    Given the mock service responds to POST "/drive/v3/files/F1/copy" with status 200 and body:
      """
      {"id": "F3"}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "file", "operation": "copy", "fileId": {"mode": "id", "value": "F1"},
        "name": "Renamed.txt", "sameFolder": false,
        "driveId": {"mode": "id", "value": "My Drive"}, "folderId": {"mode": "id", "value": "FOLDER2"},
        "options": {}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1/copy" had a JSON body matching:
      """
      {"name": "Renamed.txt", "parents": ["FOLDER2"]}
      """

  # ---- file: createFromText --------------------------------------------------

  Scenario: Create a text file from plain text uploads multipart then patches the parent folder
    Given the mock service responds to POST "/upload/drive/v3/files" with status 200 and body:
      """
      {"id": "NEWFILE"}
      """
    And the mock service responds to PATCH "/drive/v3/files/NEWFILE" with status 200 and body:
      """
      {"id": "NEWFILE", "name": "notes.txt"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "createFromText", "content": "hello world", "name": "notes.txt", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/upload/drive/v3/files" had the query parameter "uploadType" equal to "multipart"
    And the last request to "/upload/drive/v3/files" had the multipart field "data" equal to "hello world"
    And the last request to "/upload/drive/v3/files" had the multipart field "metadata" with a JSON body matching:
      """
      {"name": "notes.txt", "parents": ["root"], "mimeType": "text/plain"}
      """
    And the node "Node" outputs:
      """
      [{"id": "NEWFILE"}]
      """

  Scenario: Create a Google Document from text converts via the Docs API
    Given the mock service responds to POST "/drive/v3/files" with status 200 and body:
      """
      {"id": "DOC1"}
      """
    And the mock service responds to POST "/v1/documents/DOC1:batchUpdate" with status 200 and body:
      """
      {"documentId": "DOC1"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "createFromText", "content": "hello doc", "name": "My Doc", "options": {"convertToGoogleDocument": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files" had a JSON body matching:
      """
      {"name": "My Doc", "mimeType": "application/vnd.google-apps.document"}
      """
    And the last request to "/v1/documents/DOC1:batchUpdate" had a JSON body matching:
      """
      {"requests": [{"insertText": {"text": "hello doc"}}]}
      """
    And the node "Node" outputs:
      """
      [{"id": "DOC1"}]
      """

  # ---- file: deleteFile -------------------------------------------------------

  Scenario: Delete a file moves it to the trash by default
    Given the mock service responds to PATCH "/drive/v3/files/F1" with status 200 and body:
      """
      {}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "deleteFile", "fileId": {"mode": "id", "value": "F1"}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1" had a JSON body matching:
      """
      {"trashed": true}
      """
    And the node "Node" outputs:
      """
      [{"id": "F1", "success": true}]
      """

  Scenario: Delete a file permanently issues a DELETE
    Given the mock service responds to DELETE "/drive/v3/files/F1" with status 200
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "deleteFile", "fileId": {"mode": "id", "value": "F1"}, "options": {"deletePermanently": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 request to "/drive/v3/files/F1"

  # ---- file: download -----------------------------------------------------------

  Scenario: Download a regular file into a binary property
    Given the mock service responds to GET "/drive/v3/files/F1" with query parameter "fields" equal to "mimeType,name" with status 200 and body:
      """
      {"mimeType": "text/plain", "name": "notes.txt"}
      """
    And the mock service responds to GET "/drive/v3/files/F1" with query parameter "alt" equal to "media" with status 200 and body:
      """
      file contents here
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "download", "fileId": {"mode": "id", "value": "F1"}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" output item 0 has the binary property "data" with file name "notes.txt"

  Scenario: Download a Google Doc exports it in the requested conversion format
    Given the mock service responds to GET "/drive/v3/files/F1" with status 200 and body:
      """
      {"mimeType": "application/vnd.google-apps.document", "name": "Doc"}
      """
    And the mock service responds to GET "/drive/v3/files/F1/export" with status 200 and body:
      """
      exported content
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "download", "fileId": {"mode": "id", "value": "F1"}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1/export" had the query parameter "mimeType" equal to "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    And the node "Node" output item 0 has the binary property "data" with file name "Doc"

  # ---- file: move -----------------------------------------------------------

  Scenario: Move a file to a different folder removes the old parent
    Given the mock service responds to GET "/drive/v3/files/F1" with status 200 and body:
      """
      {"parents": ["OLDFOLDER"]}
      """
    And the mock service responds to PATCH "/drive/v3/files/F1" with status 200 and body:
      """
      {"id": "F1", "parents": ["NEWFOLDER"]}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "file", "operation": "move", "fileId": {"mode": "id", "value": "F1"},
        "driveId": {"mode": "id", "value": "My Drive"}, "folderId": {"mode": "id", "value": "NEWFOLDER"}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1" had the query parameter "addParents" equal to "NEWFOLDER"
    And the last request to "/drive/v3/files/F1" had the query parameter "removeParents" equal to "OLDFOLDER"

  # ---- file: share / folder: share --------------------------------------------

  Scenario: Share a file with a reader
    Given the mock service responds to POST "/drive/v3/files/F1/permissions" with status 200 and body:
      """
      {"id": "perm1", "type": "user", "role": "reader"}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "file", "operation": "share", "fileId": {"mode": "id", "value": "F1"},
        "permissionsUi": {"permissionsValues": {"role": "reader", "type": "user", "emailAddress": "bob@example.com"}},
        "options": {"sendNotificationEmail": true}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1/permissions" had a JSON body matching:
      """
      {"role": "reader", "type": "user", "emailAddress": "bob@example.com"}
      """
    And the last request to "/drive/v3/files/F1/permissions" had the query parameter "sendNotificationEmail" equal to "true"
    And the node "Node" outputs:
      """
      [{"id": "perm1", "type": "user", "role": "reader"}]
      """

  Scenario: Share a folder with a domain
    Given the mock service responds to POST "/drive/v3/files/FOLD1/permissions" with status 200 and body:
      """
      {"id": "perm2"}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "folder", "operation": "share", "folderNoRootId": {"mode": "id", "value": "FOLD1"},
        "permissionsUi": {"permissionsValues": {"role": "reader", "type": "domain", "domain": "example.com"}},
        "options": {}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/FOLD1/permissions" had a JSON body matching:
      """
      {"role": "reader", "type": "domain", "domain": "example.com"}
      """

  # ---- file: update -----------------------------------------------------------

  Scenario: Update a file's metadata (rename)
    Given the mock service responds to PATCH "/drive/v3/files/F1" with status 200 and body:
      """
      {"id": "F1", "name": "renamed.txt"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "update", "fileId": {"mode": "id", "value": "F1"}, "changeFileContent": false, "newUpdatedFileName": "renamed.txt", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files/F1" had a JSON body matching:
      """
      {"name": "renamed.txt"}
      """
    And the node "Node" outputs:
      """
      [{"id": "F1", "name": "renamed.txt"}]
      """

  Scenario: Update a file's content uploads the new binary then patches the mime type
    Given the mock service responds to PATCH "/upload/drive/v3/files/F1" with status 200 and body:
      """
      {}
      """
    And the mock service responds to PATCH "/drive/v3/files/F1" with status 200 and body:
      """
      {"id": "F1"}
      """
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "new file contents" and mime type "text/plain"
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "update", "fileId": {"mode": "id", "value": "F1"}, "changeFileContent": true, "inputDataFieldName": "data", "newUpdatedFileName": "", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/upload/drive/v3/files/F1" had the query parameter "uploadType" equal to "media"
    And the last request to "/drive/v3/files/F1" had a JSON body matching:
      """
      {"mimeType": "text/plain"}
      """

  # ---- file: upload -----------------------------------------------------------

  Scenario: Upload a binary file into a folder
    Given the mock service responds to POST "/upload/drive/v3/files" with status 200 and body:
      """
      {"id": "UPLOADED1"}
      """
    And the mock service responds to PATCH "/drive/v3/files/UPLOADED1" with status 200 and body:
      """
      {"id": "UPLOADED1", "name": "data.txt"}
      """
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "upload me" and mime type "text/plain"
    And the node "Node" has parameters:
      """
      {
        "resource": "file", "operation": "upload", "inputDataFieldName": "data", "name": "",
        "driveId": {"mode": "id", "value": "My Drive"}, "folderId": {"mode": "id", "value": "FOLDER9"},
        "options": {}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/upload/drive/v3/files" had the multipart field "data" equal to "upload me"
    And the last request to "/drive/v3/files/UPLOADED1" had the query parameter "addParents" equal to "FOLDER9"
    And the last request to "/drive/v3/files/UPLOADED1" had a JSON body matching:
      """
      {"mimeType": "text/plain", "name": "data.txt", "originalFilename": "data.txt"}
      """
    And the node "Node" outputs:
      """
      [{"id": "UPLOADED1", "name": "data.txt"}]
      """

  # ---- fileFolder: search -----------------------------------------------------

  Scenario: Search by name with a limit
    Given the mock service responds to GET "/drive/v3/files" with status 200 and body:
      """
      {"files": [{"id": "F1", "name": "report.pdf"}]}
      """
    And the node "Node" has parameters:
      """
      {"resource": "fileFolder", "operation": "search", "searchMethod": "name", "queryString": "report", "returnAll": false, "limit": 10, "filter": {}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files" had the query parameter "q" equal to "name contains 'report'"
    And the last request to "/drive/v3/files" had the query parameter "pageSize" equal to "10"
    And the node "Node" outputs:
      """
      [{"id": "F1", "name": "report.pdf"}]
      """

  Scenario: Search with an advanced query paginates when Return All is set
    Given the mock service responds to GET "/drive/v3/files" in order with:
      """
      [
        {"status": 200, "body": {"files": [{"id": "A"}], "nextPageToken": "page2"}},
        {"status": 200, "body": {"files": [{"id": "B"}]}}
      ]
      """
    And the node "Node" has parameters:
      """
      {"resource": "fileFolder", "operation": "search", "searchMethod": "query", "queryString": "mimeType = 'application/pdf'", "returnAll": true, "filter": {}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/drive/v3/files"
    And the node "Node" outputs:
      """
      [{"id": "A"}, {"id": "B"}]
      """

  Scenario: Search within a specific shared drive sets the drive scopes
    Given the mock service responds to GET "/drive/v3/files" with status 200 and body:
      """
      {"files": []}
      """
    And the node "Node" has parameters:
      """
      {
        "resource": "fileFolder", "operation": "search", "searchMethod": "name", "queryString": "x",
        "returnAll": false, "limit": 50,
        "filter": {"driveId": {"mode": "id", "value": "SHAREDDRIVE1"}},
        "options": {}
      }
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files" had the query parameter "driveId" equal to "SHAREDDRIVE1"
    And the last request to "/drive/v3/files" had the query parameter "corpora" equal to "drive"

  # ---- folder: create / deleteFolder ------------------------------------------

  Scenario: Create a folder
    Given the mock service responds to POST "/drive/v3/files" with status 200 and body:
      """
      {"id": "NEWFOLDER", "name": "Reports"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "folder", "operation": "create", "name": "Reports", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/files" had a JSON body matching:
      """
      {"name": "Reports", "mimeType": "application/vnd.google-apps.folder", "parents": ["root"]}
      """
    And the node "Node" outputs:
      """
      [{"id": "NEWFOLDER", "name": "Reports"}]
      """

  Scenario: Delete a folder permanently
    Given the mock service responds to DELETE "/drive/v3/files/FOLD1" with status 200
    And the node "Node" has parameters:
      """
      {"resource": "folder", "operation": "deleteFolder", "folderNoRootId": {"mode": "id", "value": "FOLD1"}, "options": {"deletePermanently": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"fileId": "FOLD1", "success": true}]
      """

  # ---- drive: create / deleteDrive / get / list / update ----------------------

  Scenario: Create a shared drive
    Given the mock service responds to POST "/drive/v3/drives" with status 200 and body:
      """
      {"id": "SD1", "name": "Team Drive"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "create", "name": "Team Drive", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/drives" had a JSON body matching:
      """
      {"name": "Team Drive"}
      """
    And the node "Node" outputs:
      """
      [{"id": "SD1", "name": "Team Drive"}]
      """

  Scenario: Delete a shared drive
    Given the mock service responds to DELETE "/drive/v3/drives/SD1" with status 200
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "deleteDrive", "driveId": {"mode": "id", "value": "SD1"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"success": true}]
      """

  Scenario: Get a shared drive
    Given the mock service responds to GET "/drive/v3/drives/SD1" with status 200 and body:
      """
      {"id": "SD1", "name": "Team Drive"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "get", "driveId": {"mode": "id", "value": "SD1"}, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"id": "SD1", "name": "Team Drive"}]
      """

  Scenario: List shared drives with a limit
    Given the mock service responds to GET "/drive/v3/drives" with status 200 and body:
      """
      {"drives": [{"id": "SD1"}, {"id": "SD2"}]}
      """
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "list", "returnAll": false, "limit": 5, "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/drives" had the query parameter "pageSize" equal to "5"
    And the node "Node" outputs:
      """
      [{"id": "SD1"}, {"id": "SD2"}]
      """

  Scenario: Update a shared drive's name
    Given the mock service responds to PATCH "/drive/v3/drives/SD1" with status 200 and body:
      """
      {"id": "SD1", "name": "Renamed Drive"}
      """
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "update", "driveId": {"mode": "id", "value": "SD1"}, "options": {"name": "Renamed Drive"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/drives/SD1" had a JSON body matching:
      """
      {"name": "Renamed Drive"}
      """

  # ---- authentication ----------------------------------------------------------

  Scenario: A missing Google Drive credential fails with a clear message
    Given the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "drive", "operation": "list", "returnAll": true}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: Clear OAuth2 authentication connects with a bearer token
    Given the credential "OAuth Drive" of type "googleDriveOAuth2Api" with the data:
      """
      {"url": "%{MOCK_URL}", "oauthTokenData": {"access_token": "drive-oauth-token"}}
      """
    And the mock service responds to GET "/drive/v3/drives" with status 200 and body:
      """
      {"drives": []}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "drive", "operation": "list", "returnAll": true}
      """
    And the node "Node" uses the "googleDriveOAuth2Api" credential "OAuth Drive"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/drive/v3/drives" had the header "authorization" equal to "Bearer drive-oauth-token"

  Scenario: A 401 triggers one OAuth2 token refresh and a retry
    Given the credential "OAuth Drive" of type "googleDriveOAuth2Api" with the data:
      """
      {
        "url": "%{MOCK_URL}",
        "accessTokenUrl": "%{MOCK_URL}/oauth2/refresh",
        "clientId": "client-1",
        "clientSecret": "secret-1",
        "oauthTokenData": {"access_token": "expired-token", "refresh_token": "refresh-1"}
      }
      """
    And the mock service responds to GET "/drive/v3/drives" with status 401 the first 1 times
    And the mock service responds to GET "/drive/v3/drives" with status 200 and body:
      """
      {"drives": []}
      """
    And the mock service responds to POST "/oauth2/refresh" with status 200 and body:
      """
      {"access_token": "fresh-token", "refresh_token": "refresh-2"}
      """
    And the node "Node" has parameters:
      """
      {"authentication": "oAuth2", "resource": "drive", "operation": "list", "returnAll": true}
      """
    And the node "Node" uses the "googleDriveOAuth2Api" credential "OAuth Drive"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 2 requests to "/drive/v3/drives"
    And the mock service received 1 request to "/oauth2/refresh"
    And the last request to "/drive/v3/drives" had the header "authorization" equal to "Bearer fresh-token"

  # ---- errors -------------------------------------------------------------------

  Scenario: A 403 from Google Drive becomes a clear error and the token never leaks
    Given the mock service responds to PATCH "/drive/v3/files/F1" with status 403 and body:
      """
      {"error": {"code": 403, "message": "The user does not have sufficient permissions for this file."}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "file", "operation": "deleteFile", "fileId": {"mode": "id", "value": "F1"}, "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "sufficient permissions"
    And the execution data does not contain "svc-token"

  Scenario: A 404 from Google Drive becomes a clear error
    Given the mock service responds to GET "/drive/v3/drives/MISSING" with status 404 and body:
      """
      {"error": {"code": 404, "message": "File not found: MISSING."}}
      """
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "get", "driveId": {"mode": "id", "value": "MISSING"}, "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "File not found"

  Scenario: continueOnFail turns a Google Drive error into an error item instead of failing the run
    Given the mock service responds to GET "/drive/v3/drives/SD1" with status 500
    And the node "Node" has parameters:
      """
      {"resource": "drive", "operation": "get", "driveId": {"mode": "id", "value": "SD1"}, "options": {}}
      """
    And the node "Node" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Node" outputs:
      """
      [{"error": "$contains:status code 500"}]
      """

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given the node "Node" has parameters:
      """
      {"resource": "comment", "operation": "list"}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation on a supported resource returns a clear message
    Given the node "Node" has parameters:
      """
      {"resource": "file", "operation": "generateContent"}
      """
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
