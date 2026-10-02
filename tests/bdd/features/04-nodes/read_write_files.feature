@spec-6.6 @phase-4 @node-read-write-file
Feature: Read/Write Files from Disk node (v1, v1.1)
  `n8n-nodes-base.readWriteFile`: reads one or more files from the disk of
  the machine running r8r into binary items (`fileSelector` supports glob
  patterns), and writes a binary property back out to disk, optionally
  appending. Access is gated by n8n-core's file-access restriction:
  `N8N_RESTRICT_FILE_ACCESS_TO` (an allow-list of directories) and
  `N8N_BLOCK_FILE_ACCESS_TO_N8N_FILES` (on by default, blocks the n8n user
  folder).

  Scenario: Reading a single file from disk returns its content as binary data
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file "hello.txt" contains:
      """
      hello world
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/hello.txt"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Read" outputs items matching:
      """
      [{"mimeType": "text/plain", "fileType": "text", "fileName": "hello.txt", "fileExtension": "txt"}]
      """
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Read": [
                {"data": {"main": [[{"binary": {"data": {"fileName": "hello.txt", "mimeType": "text/plain", "fileExtension": "txt"}}}]]}}
              ]
            }
          }
        }
      }
      """

  Scenario: A glob file selector reads every matching file into its own item
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file "multi-a.txt" contains:
      """
      AAA
      """
    And the file "multi-b.txt" contains:
      """
      BBB
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/multi-*.txt"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Read" outputs items matching:
      """
      [
        {"fileName": "multi-a.txt", "mimeType": "text/plain"},
        {"fileName": "multi-b.txt", "mimeType": "text/plain"}
      ]
      """

  Scenario: Writing a binary property creates the file on disk
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Write | readWriteFile |
    And the connections "Start -> Write"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "written content" and mime type "text/plain"
    And the node "Write" has parameters:
      """
      {"operation": "write", "fileName": "%{USER_FOLDER}/out.txt", "dataPropertyName": "data"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Write" outputs items matching:
      """
      [{"fileName": "%{USER_FOLDER}/out.txt"}]
      """
    And the file "out.txt" contains "written content"

  Scenario: Writing with append adds to the existing file instead of replacing it
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Write | readWriteFile |
    And the connections "Start -> Write"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file "append.txt" contains:
      """
      AAA
      """
    And the trigger item 0 has the binary property "data" with content "BBB" and mime type "text/plain"
    And the node "Write" has parameters:
      """
      {"operation": "write", "fileName": "%{USER_FOLDER}/append.txt", "dataPropertyName": "data", "options": {"append": true}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the file "append.txt" contains "AAA"
    And the file "append.txt" contains "BBB"

  Scenario: Writing then reading the same file back round-trips its content
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Write | readWriteFile |
      | Read  | readWriteFile |
    And the connections "Start -> Write", "Write -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "round trip content" and mime type "text/plain"
    And the node "Write" has parameters:
      """
      {"operation": "write", "fileName": "%{USER_FOLDER}/roundtrip.txt", "dataPropertyName": "data"}
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "={{ $json.fileName }}"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Read" outputs items matching:
      """
      [{"mimeType": "text/plain", "fileName": "roundtrip.txt", "fileExtension": "txt"}]
      """
    And the file "roundtrip.txt" contains "round trip content"

  Scenario: A file selector matching nothing fails on typeVersion 1.1
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/missing-*.txt"}
      """
    And the node "Read" has the property "typeVersion" set to 1.1
    When I execute the workflow
    Then the execution fails
    And the node "Read" failed with an error containing "No file(s) found"

  Scenario: A file selector matching nothing on typeVersion 1 returns no items instead of failing
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/missing-*.txt"}
      """
    And the node "Read" has the property "typeVersion" set to 1
    When I execute the workflow
    Then the execution succeeds
    And the node "Read" outputs 0 items

  Scenario: Reading a path outside N8N_RESTRICT_FILE_ACCESS_TO is rejected
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file "outside.txt" contains:
      """
      secret data
      """
    And the environment variable "N8N_RESTRICT_FILE_ACCESS_TO" is "%{USER_FOLDER}/allowed"
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/outside.txt"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Read" failed with an error containing "Access to the file is not allowed."

  Scenario: A symlink inside N8N_RESTRICT_FILE_ACCESS_TO cannot reach a file outside it
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file "outside.txt" contains:
      """
      secret data
      """
    And the file "allowed/link.txt" is a symlink to "outside.txt"
    And the environment variable "N8N_RESTRICT_FILE_ACCESS_TO" is "%{USER_FOLDER}/allowed"
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/allowed/link.txt"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Read" failed with an error containing "Access to the file is not allowed."

  Scenario: Writing through a symlinked directory inside N8N_RESTRICT_FILE_ACCESS_TO is rejected
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Write | readWriteFile |
    And the connections "Start -> Write"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "pwned" and mime type "text/plain"
    And the file "outside/keep.txt" is a symlink to "outside.txt"
    And the file "allowed/escape" is a symlink to "outside"
    And the environment variable "N8N_RESTRICT_FILE_ACCESS_TO" is "%{USER_FOLDER}/allowed"
    And the node "Write" has parameters:
      """
      {"operation": "write", "fileName": "%{USER_FOLDER}/allowed/escape/new.txt", "dataPropertyName": "data"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Write" failed with an error containing "Access to the file is not allowed."
    And the file "outside/new.txt" does not exist

  Scenario: Reading from the n8n user folder is blocked even without N8N_RESTRICT_FILE_ACCESS_TO
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Read  | readWriteFile |
    And the connections "Start -> Read"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the file ".n8n/secret.txt" contains:
      """
      instance secret
      """
    And the node "Read" has parameters:
      """
      {"operation": "read", "fileSelector": "%{USER_FOLDER}/.n8n/secret.txt"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Read" failed with an error containing "Access to the file is not allowed."
