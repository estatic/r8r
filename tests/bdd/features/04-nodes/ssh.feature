@spec-6.6 @phase-4 @node-ssh @requires-ssh
Feature: SSH node
  Execute commands and transfer files via SSH (Docker `r8r-bdd-ssh`,
  127.0.0.1:2222, user r8r / password r8r; opt in with
  R8R_BDD_INCLUDE=requires-ssh). Faithful to n8n's `Ssh.node.js`
  (typeVersion 1): resource "command" (operation "execute": command + cwd ->
  `{code, signal, stdout, stderr}`), resource "file" (upload from binary to
  path/fileName, download to binary with binaryPropertyName). Authentication
  "password" (credential `sshPassword`) or "privateKey" (credential
  `sshPrivateKey`, with optional passphrase).

  Background:
    Given the credential "SSH Password" of type "sshPassword" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "r8r"}
      """

  Scenario: Execute a command returns code, stdout and stderr
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "echo hello-r8r", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "SSH Password"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution succeeds
    And the node "Exec" outputs items matching:
      """
      [{"code": 0, "stdout": "hello-r8r", "stderr": ""}]
      """

  Scenario: Execute runs in the given working directory
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "pwd", "cwd": "/tmp"}
      """
    And the node "Exec" uses the "sshPassword" credential "SSH Password"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution succeeds
    And the node "Exec" outputs items matching:
      """
      [{"code": 0, "stdout": "/tmp"}]
      """

  Scenario: A command with a non-zero exit code does not fail the node
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "exit 7", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "SSH Password"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution succeeds
    And the node "Exec" outputs items matching:
      """
      [{"code": 7}]
      """

  Scenario: Upload a file from binary data, then download it back with the same bytes
    Given the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "round trip payload 12345" and mime type "text/plain"
    And a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Upload   | ssh           |
      | Download | ssh           |
    And the node "Upload" has parameters:
      """
      {"authentication": "password", "resource": "file", "operation": "upload", "binaryPropertyName": "data", "path": "/tmp", "options": {"fileName": "r8r-bdd-roundtrip.txt"}}
      """
    And the node "Upload" uses the "sshPassword" credential "SSH Password"
    And the node "Download" has parameters:
      """
      {"authentication": "password", "resource": "file", "operation": "download", "path": "/tmp/r8r-bdd-roundtrip.txt", "binaryPropertyName": "downloaded", "options": {}}
      """
    And the node "Download" uses the "sshPassword" credential "SSH Password"
    And the connections "Start -> Upload -> Download"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Download": [
                {"data": {"main": [[{"binary": {"downloaded": {"data": "cm91bmQgdHJpcCBwYXlsb2FkIDEyMzQ1"}}}]]}}
              ]
            }
          }
        }
      }
      """

  Scenario: Private key authentication executes a command
    Given the credential "SSH Private Key" of type "sshPrivateKey" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "privateKey": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpQIBAAKCAQEAv9jmlRSpUUKYM7QSvB8/nyihNJKxpmsqREbNceZjv54tcahX\n5ILXJ1rL8SiIKzbeqWW1OfsOTqVS9trOBUYCjisciQrQ5rNKC+wmu//XmwKFBO3Y\nUVwhQvckJ5PP+yWCnqVYtHHmgnjBzUNs2xGgYPMJbf/ERGIO1hsXlQ4+7IJ8JOYU\nsB2CiEtem+vmYmzYARFwp2wouCL0tjOpgYYbs2r3K3dIblspOBVHsdEgQ6GwX62Z\nXcXeEVxDJV/w7H9lreJbEnJRLjd3Nplh839BUyoVesF258xc2Td9eH+TEIp0itPq\nhSPqKTgbXSYGnVEFZWPUl+KoqqnAmYlT0+widwIDAQABAoIBABG0UpMvdPCq1KDw\n3Um/GH/3n+jaIMra2Ou0HQGF4W6tikQS5QmRiYl4N5npw6c5SRMY78CKCqf3WdCu\nIJUcAOBd2iRLezvPD+ZJxMlUsvD7Kotp56yac/daF1wPqAHlVEiPub/tQviHbFsk\n/ca5ubM4uoYMNHCbHANmPKbkyanJt/tEAbwoli6UWzIKgMsjlZmRN0XIa3vygVP/\naIQXMUShHIS79Ac+K8GvaJ9VveW0F8OW7kmfb5pRvh7RMSDjW74AF5KUL4jPGGnt\nGnBpOEXSNsY5k+qMCzp9nh5uh5Ry+9LNt0dyfLsmLIdT1/0OsTKdWFBloJnXT9oJ\nKBoQpgECgYEA5s3oGs4AXHMT3+vhqDLZfN6okiYdcmHU/08lzWcSmf6PEwFizX9q\nDQHl9q4RqbxlhAUk7ri9lLXvlo3vht0Ni3chNnjWv3K5eRTMlUC/kZ4qjhS1jzcQ\n8APYT+/dKYcRS+jmn72DR3iW6Jg2/BU4jOh+kZGTBt7TQAgXGboh6/8CgYEA1MpL\nTzWn4rtft05GZeeaGOJ1s4rNTdshYCHxkBp2qoSsb24fn16YQ3+IRui6KYPDuu2N\nU+7y5r4+8PhQlf5S13KHV0fXE+QvYU5Dz4MvvfhHdhw2JHfYTWdIdtgDrn2DTlBA\n2BsxY85OP74lP/8CMf+qjkt6QaZBGORd0asHKYkCgYEAo6p1obRcyzILcOkq3oQY\nd5TIwsJmdTrsuJLegpZJuYuq11ZPQtvMTyb+dHuhKNTgw8qGEwZL4U1WdVN2/BJ6\nzWAP3Dm6Ro5K+ZKHNHtynQcktBX5XXI1/40qchzsnssZt9OS3smbcy4PDScLLpy5\nuioPvpgVQ3T1Jl3WS7cS4O8CgYEAxconiD/yhqDuTl+WtiA6L0lIaJPrU6QM+ON0\nOyMuDD/4XDc7c5Kk65C+plKqv+33YGhwxoTECVDrBmd52IImlJajULC5LYcbt1Hd\nXCSvmuAN5K5CcVFooEIRrE4L1gRaqc+VBor4NLJOL5fZ3gt2Ce2ApnVn9V/JAI88\n2agkpykCgYEAo6ebueege94LW9ZcpuRB7tt6nvLsNTn1bZHveZmrEPAIzei336yh\ntozDP8IKIqYfA6xCU13TK8fQ5DOLgGCQGK3YhW3QmNBuusY1Dl5Q/qtd29pw84Qw\nPJhiKb3T+wYpM3pw58e7Vyni1r2PiLoTBCXxLGFEUQUglNCz5lqhy9A=\n-----END RSA PRIVATE KEY-----\n"}
      """
    And a workflow with nodes:
      | name       | type          |
      | Start      | manualTrigger |
      | AddKey     | ssh           |
      | ExecViaKey | ssh           |
    And the node "AddKey" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "mkdir -p ~/.ssh && echo 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC/2OaVFKlRQpgztBK8Hz+fKKE0krGmaypERs1x5mO/ni1xqFfkgtcnWsvxKIgrNt6pZbU5+w5OpVL22s4FRgKOKxyJCtDms0oL7Ca7/9ebAoUE7dhRXCFC9yQnk8/7JYKepVi0ceaCeMHNQ2zbEaBg8wlt/8REYg7WGxeVDj7sgnwk5hSwHYKIS16b6+ZibNgBEXCnbCi4IvS2M6mBhhuzavcrd0huWyk4FUex0SBDobBfrZldxd4RXEMlX/Dsf2Wt4lsSclEuN3c2mWHzf0FTKhV6wXbnzFzZN314f5MQinSK0+qFI+opOBtdJgadUQVlY9SX4qiqqcCZiVPT7CJ3 r8r-bdd-test' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys", "cwd": "/"}
      """
    And the node "AddKey" uses the "sshPassword" credential "SSH Password"
    And the node "ExecViaKey" has parameters:
      """
      {"authentication": "privateKey", "resource": "command", "operation": "execute", "command": "echo via-private-key", "cwd": "/"}
      """
    And the node "ExecViaKey" uses the "sshPrivateKey" credential "SSH Private Key"
    And the connections "Start -> AddKey -> ExecViaKey"
    When I execute the workflow
    Then the execution succeeds
    And the node "ExecViaKey" outputs items matching:
      """
      [{"code": 0, "stdout": "via-private-key"}]
      """

  @security
  Scenario: A wrong password produces a clear error without leaking the password
    Given the credential "Bad SSH Password" of type "sshPassword" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "sUp3rWr0ngPassphrase!"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "echo hi", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "Bad SSH Password"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution fails
    And the node "Exec" failed with an error containing "SSH"
    And the execution data does not contain "sUp3rWr0ngPassphrase!"

  Scenario: An unreachable host produces a clear error
    Given the credential "Unreachable SSH" of type "sshPassword" with the data:
      """
      {"host": "127.0.0.1", "port": 2, "username": "r8r", "password": "r8r"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "echo hi", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "Unreachable SSH"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution fails
    And the node "Exec" failed with an error containing "SSH"

  Scenario: onError "continueRegularOutput" passes an error item downstream instead of failing
    Given the credential "Bad SSH Password 2" of type "sshPassword" with the data:
      """
      {"host": "127.0.0.1", "port": 2222, "username": "r8r", "password": "totally-wrong"}
      """
    And a workflow with nodes:
      | name  | type          | onError               |
      | Start | manualTrigger |                        |
      | Exec  | ssh           | continueRegularOutput  |
      | After | noOp          |                        |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "echo hi", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "Bad SSH Password 2"
    And the connections "Start -> Exec -> After"
    When I execute the workflow
    Then the execution succeeds
    And the node "After" outputs items matching:
      """
      [{"error": "$nonempty"}]
      """

  @security
  Scenario: The password credential never appears in execution data on success
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Exec  | ssh           |
    And the node "Exec" has parameters:
      """
      {"authentication": "password", "resource": "command", "operation": "execute", "command": "echo check-secrets", "cwd": "/"}
      """
    And the node "Exec" uses the "sshPassword" credential "SSH Password"
    And the connections "Start -> Exec"
    When I execute the workflow
    Then the execution succeeds
    And the execution data does not contain "\"password\":\"r8r\""
