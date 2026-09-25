@spec-6.5 @spec-4.1 @phase-3
Feature: Credentials encrypted by n8n stay usable
  n8n stores credential data as CryptoJS AES (OpenSSL "Salted__" format,
  EVP_BytesToKey/MD5) keyed by N8N_ENCRYPTION_KEY. r8r must read that format
  with the same key (goal G3), and while migration is being piloted must
  keep writing data n8n can still read, so rollback stays possible.

  Scenario: Credentials exported from n8n import and decrypt with the same key
    Given an n8n credentials export "n8n-credentials.json" encrypted with the key "default":
      | id   | name        | type           | data                                               |
      | c-1  | Stripe      | httpHeaderAuth | {"name": "Authorization", "value": "Bearer sk_1"}  |
      | c-2  | Legacy FTP  | httpBasicAuth  | {"user": "ftp", "password": "hunter2"}             |
    When I run "r8r import:credentials --input=n8n-credentials.json"
    Then the command succeeds
    When I run "r8r export:credentials --all --decrypted --output=plain.json"
    Then the command succeeds
    And the file "plain.json" has the credential "Stripe" with the decrypted data:
      """
      {"name": "Authorization", "value": "Bearer sk_1"}
      """
    And the file "plain.json" has the credential "Legacy FTP" with the decrypted data:
      """
      {"user": "ftp", "password": "hunter2"}
      """

  Scenario: Encrypted exports can be decrypted by n8n
    Given an n8n credentials export "n8n-credentials.json" encrypted with the key "default":
      | id   | name   | type           | data                                        |
      | c-1  | Stripe | httpHeaderAuth | {"name": "X-Key", "value": "sk_live_123"}   |
    And I successfully run "r8r import:credentials --input=n8n-credentials.json"
    When I run "r8r export:credentials --all --output=encrypted.json"
    Then the command succeeds
    And the file "encrypted.json" does not contain "sk_live_123"
    And the file "encrypted.json" has the credential "Stripe" whose data n8n can decrypt with the key "default" to:
      """
      {"name": "X-Key", "value": "sk_live_123"}
      """

  Scenario: Credentials created in r8r are readable by n8n
    Given the credential "Created here" of type "httpHeaderAuth" with the data:
      """
      {"name": "X-Key", "value": "made-in-r8r"}
      """
    When I run "r8r export:credentials --all --output=encrypted.json"
    Then the file "encrypted.json" has the credential "Created here" whose data n8n can decrypt with the key "default" to:
      """
      {"name": "X-Key", "value": "made-in-r8r"}
      """

  Scenario: A different encryption key cannot decrypt imported credentials
    Given an n8n credentials export "foreign.json" encrypted with the key "someone-elses-key":
      | id   | name    | type           | data                                  |
      | c-9  | Foreign | httpHeaderAuth | {"name": "X-Key", "value": "nope"}    |
    And I successfully run "r8r import:credentials --input=foreign.json"
    When I run "r8r export:credentials --all --decrypted --output=plain.json"
    Then the command fails
    And the command output contains "decrypt"

  Scenario: An imported n8n credential works in a workflow
    Given a mock HTTP service
    And an n8n credentials export "n8n-credentials.json" encrypted with the key "default":
      | id       | name    | type           | data                                          |
      | cred-42  | Partner | httpHeaderAuth | {"name": "X-Partner", "value": "p-key-42"}    |
    And I successfully run "r8r import:credentials --input=n8n-credentials.json"
    And the mock service responds to GET "/partner" with status 200
    And a workflow with nodes:
      | name  | type          | parameters                                                                                               | credentials                                               |
      | Start | manualTrigger |                                                                                                          |                                                           |
      | Call  | httpRequest   | {"url": "%{MOCK_URL}/partner", "authentication": "genericCredentialType", "genericAuthType": "httpHeaderAuth", "options": {}} | {"httpHeaderAuth": {"id": "cred-42", "name": "Partner"}} |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/partner" had the header "X-Partner" equal to "p-key-42"

  Scenario: The server refuses to start with a key that does not match existing data
    Given an n8n credentials export "n8n-credentials.json" encrypted with the key "default":
      | id  | name   | type           | data                                   |
      | c-1 | Stripe | httpHeaderAuth | {"name": "X-Key", "value": "sk"}       |
    And I successfully run "r8r import:credentials --input=n8n-credentials.json"
    And the environment variable "N8N_ENCRYPTION_KEY" is "a-different-key"
    When I start the r8r server
    Then the server fails to start with a message containing "N8N_ENCRYPTION_KEY"
