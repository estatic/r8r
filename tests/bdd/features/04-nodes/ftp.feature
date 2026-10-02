@spec-6.6 @phase-4 @node-ftp
Feature: FTP node
  Transfer files via FTP or SFTP. Faithful to n8n's `Ftp.node.js`
  (typeVersion 1/1.1): operations delete (file/folder, recursive), download
  (-> binary), list (recursive option), rename (createDirectories option),
  upload (binary or text content, createDirectories). FTP scenarios run
  against Docker `r8r-bdd-ftp` (127.0.0.1:2121, user/password r8r/r8r,
  passive ports 30000-30009; opt in with R8R_BDD_INCLUDE=requires-ftp) via
  the `ftp` credential. SFTP scenarios reuse the SSH server, Docker
  `r8r-bdd-ssh` (127.0.0.1:2222, user/password r8r/r8r; opt in with
  R8R_BDD_INCLUDE=requires-ssh) via the `sftp` credential. Each scenario
  uses a uniquely named path and cleans up after itself.

  @requires-ftp
  Scenario: Upload text content over FTP, then download it back with the same bytes
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name     | type |
      | Start    | manualTrigger |
      | Upload   | ftp |
      | Download | ftp |
      | Cleanup  | ftp |
    And the node "Upload" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-text-1.txt", "binaryData": false, "fileContent": "hello from r8r over ftp", "options": {}}
      """
    And the node "Upload" uses the "ftp" credential "FTP Server"
    And the node "Download" has parameters:
      """
      {"protocol": "ftp", "operation": "download", "path": "/r8r-bdd-ftp-text-1.txt", "binaryPropertyName": "downloaded", "options": {}}
      """
    And the node "Download" uses the "ftp" credential "FTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-text-1.txt", "options": {}}
      """
    And the node "Cleanup" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Upload -> Download -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Download": [
                {"data": {"main": [[{"binary": {"downloaded": {"data": "aGVsbG8gZnJvbSByOHIgb3ZlciBmdHA="}}}]]}}
              ]
            }
          }
        }
      }
      """

  @requires-ftp
  Scenario: Upload binary content over FTP, then download it back with the same bytes
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name     | type |
      | Start    | manualTrigger |
      | Upload   | ftp |
      | Download | ftp |
      | Cleanup  | ftp |
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "binary ftp round trip 42" and mime type "application/octet-stream"
    And the node "Upload" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-bin-1.bin", "binaryData": true, "binaryPropertyName": "data", "options": {}}
      """
    And the node "Upload" uses the "ftp" credential "FTP Server"
    And the node "Download" has parameters:
      """
      {"protocol": "ftp", "operation": "download", "path": "/r8r-bdd-ftp-bin-1.bin", "binaryPropertyName": "downloaded", "options": {}}
      """
    And the node "Download" uses the "ftp" credential "FTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-bin-1.bin", "options": {}}
      """
    And the node "Cleanup" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Upload -> Download -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Download": [
                {"data": {"main": [[{"binary": {"downloaded": {"data": "YmluYXJ5IGZ0cCByb3VuZCB0cmlwIDQy"}}}]]}}
              ]
            }
          }
        }
      }
      """

  @requires-ftp
  Scenario: List a directory shows name, type and size
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | List    | ftp |
      | Cleanup | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-list/inside.txt", "binaryData": false, "fileContent": "listed file", "options": {"createDirectories": true}}
      """
    And the node "Setup" uses the "ftp" credential "FTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/r8r-bdd-ftp-list", "recursive": false, "options": {}}
      """
    And the node "List" uses the "ftp" credential "FTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-list", "options": {"folder": true, "recursive": true}}
      """
    And the node "Cleanup" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Setup -> List -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "inside.txt", "type": "-", "size": 11, "path": "/r8r-bdd-ftp-list/inside.txt"}]
      """

  @requires-ftp
  Scenario: Recursive list finds files in nested folders
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | List    | ftp |
      | OneItem | limit |
      | Cleanup | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-rec/nested/deep.txt", "binaryData": false, "fileContent": "deep file", "options": {"createDirectories": true}}
      """
    And the node "Setup" uses the "ftp" credential "FTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/r8r-bdd-ftp-rec", "recursive": true, "options": {}}
      """
    And the node "List" uses the "ftp" credential "FTP Server"
    And the node "OneItem" has parameters:
      """
      {"maxItems": 1}
      """
    And the node "Cleanup" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-rec", "options": {"folder": true, "recursive": true}}
      """
    And the node "Cleanup" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Setup -> List -> OneItem -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "nested", "type": "d"}, {"name": "deep.txt", "type": "-", "path": "/r8r-bdd-ftp-rec/nested/deep.txt"}]
      """

  @requires-ftp
  Scenario: Rename with createDirectories creates the missing destination folder
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | Rename  | ftp |
      | List    | ftp |
      | Cleanup | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-ren-1.txt", "binaryData": false, "fileContent": "to be renamed", "options": {}}
      """
    And the node "Setup" uses the "ftp" credential "FTP Server"
    And the node "Rename" has parameters:
      """
      {"protocol": "ftp", "operation": "rename", "oldPath": "/r8r-bdd-ftp-ren-1.txt", "newPath": "/r8r-bdd-ftp-ren-dir/moved.txt", "options": {"createDirectories": true}}
      """
    And the node "Rename" uses the "ftp" credential "FTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/r8r-bdd-ftp-ren-dir", "recursive": false, "options": {}}
      """
    And the node "List" uses the "ftp" credential "FTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-ren-dir", "options": {"folder": true, "recursive": true}}
      """
    And the node "Cleanup" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Setup -> Rename -> List -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "moved.txt", "type": "-"}]
      """

  @requires-ftp
  Scenario: Delete removes a file
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | Delete  | ftp |
      | List    | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-del-1.txt", "binaryData": false, "fileContent": "gone soon", "options": {}}
      """
    And the node "Setup" uses the "ftp" credential "FTP Server"
    And the node "Delete" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-del-1.txt", "options": {}}
      """
    And the node "Delete" uses the "ftp" credential "FTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/", "recursive": false, "options": {}}
      """
    And the node "List" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Setup -> Delete -> List"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs items matching:
      """
      [{"success": true}]
      """

  @requires-ftp
  Scenario: Delete a folder recursively removes it and everything inside
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | Delete  | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "ftp", "operation": "upload", "path": "/r8r-bdd-ftp-delrec/a/b.txt", "binaryData": false, "fileContent": "inside a nested folder", "options": {"createDirectories": true}}
      """
    And the node "Setup" uses the "ftp" credential "FTP Server"
    And the node "Delete" has parameters:
      """
      {"protocol": "ftp", "operation": "delete", "path": "/r8r-bdd-ftp-delrec", "options": {"folder": true, "recursive": true}}
      """
    And the node "Delete" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Setup -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs items matching:
      """
      [{"success": true}]
      """

  @requires-ftp
  Scenario: onError "continueRegularOutput" passes an error item downstream instead of failing
    Given the credential "FTP Server" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name     | type | onError               |
      | Start    | manualTrigger | |
      | Download | ftp  | continueRegularOutput |
      | After    | noOp | |
    And the node "Download" has parameters:
      """
      {"protocol": "ftp", "operation": "download", "path": "/does-not-exist-r8r-bdd.txt", "binaryPropertyName": "data", "options": {}}
      """
    And the node "Download" uses the "ftp" credential "FTP Server"
    And the connections "Start -> Download -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  @requires-ftp @security
  Scenario: A wrong FTP password produces a clear error without leaking the password
    Given the credential "Bad FTP" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2121, "username": "r8r", "password": "sUp3rWr0ngFtpPassphrase!"}
      """
    And a workflow with nodes:
      | name  | type |
      | Start | manualTrigger |
      | List  | ftp |
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/", "recursive": false, "options": {}}
      """
    And the node "List" uses the "ftp" credential "Bad FTP"
    And the connections "Start -> List"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sUp3rWr0ngFtpPassphrase!"

  @requires-ftp
  Scenario: An unreachable FTP host produces a clear error
    Given the credential "Unreachable FTP" of type "ftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name  | type |
      | Start | manualTrigger |
      | List  | ftp |
    And the node "List" has parameters:
      """
      {"protocol": "ftp", "operation": "list", "path": "/", "recursive": false, "options": {}}
      """
    And the node "List" uses the "ftp" credential "Unreachable FTP"
    And the connections "Start -> List"
    When I execute the workflow
    Then the execution fails

  # ---- SFTP (protocol: sftp, over the SSH server) -----------------------

  @requires-ssh
  Scenario: Upload content over SFTP, then download it back with the same bytes
    Given the credential "SFTP Server" of type "sftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name     | type |
      | Start    | manualTrigger |
      | Upload   | ftp |
      | Download | ftp |
      | Cleanup  | ftp |
    And the node "Upload" has parameters:
      """
      {"protocol": "sftp", "operation": "upload", "path": "/tmp/r8r-bdd-sftp-text-1.txt", "binaryData": false, "fileContent": "hello from r8r over sftp", "options": {}}
      """
    And the node "Upload" uses the "sftp" credential "SFTP Server"
    And the node "Download" has parameters:
      """
      {"protocol": "sftp", "operation": "download", "path": "/tmp/r8r-bdd-sftp-text-1.txt", "binaryPropertyName": "downloaded", "options": {}}
      """
    And the node "Download" uses the "sftp" credential "SFTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "sftp", "operation": "delete", "path": "/tmp/r8r-bdd-sftp-text-1.txt", "options": {}}
      """
    And the node "Cleanup" uses the "sftp" credential "SFTP Server"
    And the connections "Start -> Upload -> Download -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Download": [
                {"data": {"main": [[{"binary": {"downloaded": {"data": "aGVsbG8gZnJvbSByOHIgb3ZlciBzZnRw"}}}]]}}
              ]
            }
          }
        }
      }
      """

  @requires-ssh
  Scenario: List a directory over SFTP shows name, type and size
    Given the credential "SFTP Server" of type "sftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | List    | ftp |
      | Cleanup | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "sftp", "operation": "upload", "path": "/tmp/r8r-bdd-sftp-list/inside.txt", "binaryData": false, "fileContent": "listed file", "options": {"createDirectories": true}}
      """
    And the node "Setup" uses the "sftp" credential "SFTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "sftp", "operation": "list", "path": "/tmp/r8r-bdd-sftp-list", "recursive": false, "options": {}}
      """
    And the node "List" uses the "sftp" credential "SFTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "sftp", "operation": "delete", "path": "/tmp/r8r-bdd-sftp-list", "options": {"folder": true, "recursive": true}}
      """
    And the node "Cleanup" uses the "sftp" credential "SFTP Server"
    And the connections "Start -> Setup -> List -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "inside.txt", "type": "-", "size": 11, "path": "/tmp/r8r-bdd-sftp-list/inside.txt"}]
      """

  @requires-ssh
  Scenario: Delete a folder recursively over SFTP removes it and everything inside
    Given the credential "SFTP Server" of type "sftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | Delete  | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "sftp", "operation": "upload", "path": "/tmp/r8r-bdd-sftp-delrec/a/b.txt", "binaryData": false, "fileContent": "inside a nested folder", "options": {"createDirectories": true}}
      """
    And the node "Setup" uses the "sftp" credential "SFTP Server"
    And the node "Delete" has parameters:
      """
      {"protocol": "sftp", "operation": "delete", "path": "/tmp/r8r-bdd-sftp-delrec", "options": {"folder": true, "recursive": true}}
      """
    And the node "Delete" uses the "sftp" credential "SFTP Server"
    And the connections "Start -> Setup -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs items matching:
      """
      [{"success": true}]
      """

  @requires-ssh
  Scenario: Rename over SFTP with createDirectories creates the missing destination folder
    Given the credential "SFTP Server" of type "sftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name    | type |
      | Start   | manualTrigger |
      | Setup   | ftp |
      | Rename  | ftp |
      | List    | ftp |
      | Cleanup | ftp |
    And the node "Setup" has parameters:
      """
      {"protocol": "sftp", "operation": "upload", "path": "/tmp/r8r-bdd-sftp-ren-1.txt", "binaryData": false, "fileContent": "to be renamed", "options": {}}
      """
    And the node "Setup" uses the "sftp" credential "SFTP Server"
    And the node "Rename" has parameters:
      """
      {"protocol": "sftp", "operation": "rename", "oldPath": "/tmp/r8r-bdd-sftp-ren-1.txt", "newPath": "/tmp/r8r-bdd-sftp-ren-dir/moved.txt", "options": {"createDirectories": true}}
      """
    And the node "Rename" uses the "sftp" credential "SFTP Server"
    And the node "List" has parameters:
      """
      {"protocol": "sftp", "operation": "list", "path": "/tmp/r8r-bdd-sftp-ren-dir", "recursive": false, "options": {}}
      """
    And the node "List" uses the "sftp" credential "SFTP Server"
    And the node "Cleanup" has parameters:
      """
      {"protocol": "sftp", "operation": "delete", "path": "/tmp/r8r-bdd-sftp-ren-dir", "options": {"folder": true, "recursive": true}}
      """
    And the node "Cleanup" uses the "sftp" credential "SFTP Server"
    And the connections "Start -> Setup -> Rename -> List -> Cleanup"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs items matching:
      """
      [{"name": "moved.txt", "type": "-"}]
      """

  @requires-ssh @security
  Scenario: A wrong SFTP password produces a clear error without leaking the password
    Given the credential "Bad SFTP" of type "sftp" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "sUp3rWr0ngSftpPassphrase!"}
      """
    And a workflow with nodes:
      | name  | type |
      | Start | manualTrigger |
      | List  | ftp |
    And the node "List" has parameters:
      """
      {"protocol": "sftp", "operation": "list", "path": "/tmp", "recursive": false, "options": {}}
      """
    And the node "List" uses the "sftp" credential "Bad SFTP"
    And the connections "Start -> List"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sUp3rWr0ngSftpPassphrase!"
