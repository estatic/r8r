@spec-8.2 @spec-2.5 @security
Feature: Isolation and hardening
  No user code or expression reaches host memory, files or network except
  through declared capabilities (goal G6). Dangerous nodes are off by
  default, as in n8n 2.0, and outbound HTTP is guarded against SSRF.

  @phase-1
  Scenario: Execute Command is disabled by default
    Given a workflow with nodes:
      | name  | type           | parameters                           |
      | Start | manualTrigger  |                                      |
      | Shell | executeCommand | {"command": "printf 'pw%s' ned"}     |
    And the connections "Start -> Shell"
    When I execute the workflow
    Then the command fails
    And the command output does not contain "pwned"
    And the command output contains "n8n-nodes-base.executeCommand"

  @phase-1
  Scenario: Execute Command can be enabled explicitly
    Given the environment variable "NODES_EXCLUDE" is "[]"
    And a workflow with nodes:
      | name  | type           | parameters                  |
      | Start | manualTrigger  |                             |
      | Shell | executeCommand | {"command": "echo hello"}   |
    And the connections "Start -> Shell"
    When I execute the workflow
    Then the execution succeeds
    And the node "Shell" outputs items matching:
      """
      [{"stdout": "hello", "exitCode": 0}]
      """

  @phase-2
  Scenario: Local File Trigger is disabled by default
    Given a running r8r server with an owner and an API key
    And a workflow named "Watch files" with nodes:
      | name  | type             | parameters                                                  |
      | Watch | localFileTrigger | {"triggerOn": "folder", "path": "/tmp", "events": ["add"]}  |
    When I activate the workflow
    Then the response status is a client error

  # The SSRF guard is new in the spec (§5.3); n8n 2.35 has none.
  @phase-1 @beyond-n8n
  Scenario: HTTP requests to private networks are blocked by default
    Given the environment variable "R8R_SSRF_ALLOWED_HOSTS" is not set
    And a mock HTTP service
    And the mock service responds to GET "/internal" with status 200
    And a workflow with nodes:
      | name  | type          | parameters                                     |
      | Start | manualTrigger |                                                |
      | Call  | httpRequest   | {"url": "%{MOCK_URL}/internal", "options": {}} |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution fails
    And the mock service received no requests

  @phase-1 @beyond-n8n
  Scenario Outline: Well-known internal targets are blocked
    Given the environment variable "R8R_SSRF_ALLOWED_HOSTS" is not set
    And a workflow with nodes:
      | name  | type          | parameters                                       |
      | Start | manualTrigger |                                                  |
      | Call  | httpRequest   | {"url": "<url>", "options": {"timeout": 2000}}   |
    And the connections "Start -> Call"
    When I execute the workflow
    Then the execution fails
    And the node "Call" failed with an error containing "blocked"

    Examples:
      | url                                          |
      | http://169.254.169.254/latest/meta-data/     |
      | http://127.0.0.1:1/                          |
      | http://[::1]:1/                              |
      | http://10.0.0.1/                             |
      | http://localhost:1/                          |

  @phase-3
  Scenario: The Code node has no network access
    Given a mock HTTP service
    And the mock service responds to GET "/exfiltrate" with status 200
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the node "Code" runs the JavaScript:
      """
      await fetch('%{MOCK_URL}/exfiltrate');
      return [{ json: { leaked: true } }];
      """
    And the connections "Start -> Code"
    When I execute the workflow
    Then the execution fails
    And the mock service received no requests

  @phase-3
  Scenario Outline: The Code node cannot load host modules unless allowed
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the node "Code" runs the JavaScript:
      """
      const m = require('<module>');
      return [{ json: { loaded: typeof m } }];
      """
    And the connections "Start -> Code"
    When I execute the workflow
    Then the execution fails
    And the node "Code" failed with an error containing "<module>"

    Examples:
      | module        |
      | fs            |
      | child_process |
      | net           |
      | http          |

  @phase-3
  Scenario: The Code node cannot read the host's environment
    Given the environment variable "BDD_HOST_SECRET" is "host-only-value"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the node "Code" runs the JavaScript:
      """
      let found = 'absent';
      try { found = process.env.BDD_HOST_SECRET ?? 'absent'; } catch (e) { found = 'blocked'; }
      return [{ json: { found } }];
      """
    And the connections "Start -> Code"
    When I execute the workflow
    Then the execution data does not contain "host-only-value"

  @phase-3
  Scenario: A runaway Code node is stopped by the task timeout
    Given the environment variable "N8N_RUNNERS_TASK_TIMEOUT" is "2"
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Code  | code          |
    And the node "Code" runs the JavaScript:
      """
      while (true) {}
      """
    And the connections "Start -> Code"
    When I execute the workflow
    Then the execution fails
    And the node "Code" failed with an error containing "timed out"
    And the command finished within 15000 ms

  @phase-3
  Scenario: A Code node that exhausts its memory fails without taking r8r down
    Given a running r8r server with an owner and an API key
    And a workflow named "Memory hog" with nodes:
      | name    | type    | parameters                                                                         |
      | Webhook | webhook | {"httpMethod": "GET", "path": "hog", "responseMode": "lastNode", "options": {}}    |
      | Code    | code    |                                                                                    |
    And the node "Code" runs the JavaScript:
      """
      const hog = [];
      while (true) { hog.push(new Array(1e6).fill('x')); }
      """
    And the connections "Webhook -> Code"
    And the workflow is active
    When I send a GET request to "/webhook/hog"
    Then the response status is 500
    When I send a GET request to "/healthz"
    Then the response status is 200

  # n8n 2.35 did not rate-limit repeated failed logins in this setup; the
  # spec (§8.2) requires rate limits on all public endpoints.
  @phase-2 @beyond-n8n
  Scenario: The editor API rate-limits login attempts
    Given a running r8r server with an owner account
    When I send 30 POST requests to "/rest/login" with concurrency 5 and body:
      """
      {"emailOrLdapLoginId": "owner@example.com", "password": "wrong-password"}
      """
    Then some load responses had the status 429
