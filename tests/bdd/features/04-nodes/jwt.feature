@spec-6.6 @phase-4 @node-jwt
Feature: JWT node
  Signs, decodes and verifies JSON Web Tokens against the `jwtAuth`
  credential (passphrase secrets for HS*, PEM keys for RS*/ES*).

  Scenario: Signing produces a well-formed token
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Sign  | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": false, "claims": {}, "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the connections "Start -> Sign"
    When I execute the workflow
    Then the execution succeeds
    And the field "token" of item 0 from the node "Sign" is "$regex:^[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+$"

  Scenario: Sign then decode round trip returns the signed claims
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Decode | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 42, \"role\": \"admin\"}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the node "Decode" has parameters:
      """
      {"operation": "decode", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Decode" uses the "jwtAuth" credential "HS Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Decode
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Decode" outputs:
      """
      [{"payload": {"user_id": 42, "role": "admin"}}]
      """

  Scenario: Sign then verify round trip accepts a valid token
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 42}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "HS Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Verify" outputs:
      """
      [{"payload": {"user_id": 42}}]
      """

  Scenario: Verifying with the wrong secret fails with a clear error and never leaks either secret
    Given the credential "Right Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "correct-horse-battery-staple", "algorithm": "HS256"}
      """
    And the credential "Wrong Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "not-the-right-one", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 42}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "Right Secret"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "Wrong Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution fails
    And the node "Verify" failed with an error containing "verified"
    And the execution data does not contain "correct-horse-battery-staple"
    And the execution data does not contain "not-the-right-one"

  Scenario: Verifying an expired token fails
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"exp\": 1000000000}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "HS Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution fails
    And the node "Verify" failed with an error containing "expired"

  Scenario: ignoreExpiration lets verify accept an expired token
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"exp\": 1000000000, \"user_id\": 7}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {"ignoreExpiration": true}}
      """
    And the node "Verify" uses the "jwtAuth" credential "HS Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Verify" outputs:
      """
      [{"payload": {"exp": 1000000000, "user_id": 7}}]
      """

  Scenario Outline: HMAC round trip for every SHA variant
    Given the credential "HS Cred" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "shared-secret", "algorithm": "<algorithm>"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"alg_under_test\": \"<algorithm>\"}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Cred"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "HS Cred"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution succeeds
    And the field "payload.alg_under_test" of item 0 from the node "Verify" is "<algorithm>"

    Examples:
      | algorithm |
      | HS256     |
      | HS384     |
      | HS512     |

  Scenario: RS256 round trip using a PEM key pair credential
    Given the credential "RSA Cred" of type "jwtAuth" with the data:
      """
      {"keyType": "pemKey", "privateKey": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCgHQNrUetQxEOs\nNfsDt8iMM4/YRAkvAXJQNN7J03CZrB5qg/4NE9yhLA/Iy+VMX+5pBrxoXn5ScTMD\ns0g1qtRR5D2tOLsTjKC+m7sOvVzRh4vJWAXAmaWxfrCg59M1AkYvbIDHs3D5hnTq\nG2ANmA5C9l5zYCeQRH3Niama6ZcI+aaOlwGWklVfTCOBlbIjhqGTr1XYo+vEu+d7\nCZRUB1AR8AphEUQKKxVzQCftnKQAt6lIYWj0Ivt57JtYeKtmvyM1a/nIm/7VH0UH\nctZQljtVKShYF/Mx0bgugJRzaXwFdoKHHddP+eZOUlxmrYdNM4LmU+KofnbMwUpN\nXnRq7OWlAgMBAAECggEAI7NMaZ/GqQfV8f5IRaQzKIWvr2A+Lvey2izrtxM44l7l\nGklrelWBJoO5UkOaUTz7nvnkRR46qt24Kv1M3sCEHm1WjjrdxaZfyhhVrVvuJ/8B\n3WdNwwvPTOQLdJk0N/fVl+nozf5V9KE1DOUFNgq/NVGTLkUUlT+cUFm/Uj3+0f5k\nCFcD/K8SnaukzCRXhXD82j+/6dtAaPyjkEHNKHYofwzvQPtYTdpwYS0Dts9hXmyb\n3g7DU2ewyPevWZSoZ4n9wKTpQ6GamGTdJEtu40VIJGIxQ/yBQYz7LXm1X/EriDMJ\nex4hoTC18Gvtjyo7uFA+nINcwqpG/V0W7D3+CNSdaQKBgQDNVXwE7rn9DOhlzHMm\nFkcHe7p3tgcCts/NC775Kyw+So9+qFRQRdQ1Hminx5w4a3+fR6eVGMf57Xl2AkGy\nat8F6KXn3iXT4qml/J/arkgGsd9YrUBAtN173owdEoNJP1wE4ho7Q58Y+U+zQ51D\n/SZYpRhjiAK+1eJpJWJRfXzGiQKBgQDHnwnfu5Vh4Of1BMTuhVK6O3lxolYYF8w3\nx+H65EN2EjXlG/EYw3RsG9QHhTqF6JVM6vyqFZjRfcuz9DET5fcn0DKvR3D7iVeU\ne/2pCHE+uvEZ9Su0gHZiXT68c/1c+9C7nXqexW16uTm5G7WOZMpEEaAIdRp/toEu\n5AWKDS8fPQKBgDxYvFtCwhyx93c7sDfoYjW70mCueb79dXMg2Z6nZphkF2o1FJqG\n+0glSMLOsoYOafKo/4KdRuCYP5NENIS4ThWRe3j63Ak623syFNUTVY3KJwcL3A9o\nWJO4I1vD/hu/6E5zGRyD0jVnyFm6LHU36FYzJ0jRR2VIvQMD/rJOfCZpAoGBAKnf\nIu3rmXGTjJCrIFLBzeaBGhWjSZRzG+wUArAYc3gUgxyWrJKgMYCWJdbIf0bY58Ru\n69hpTIRpgmF+2gzO04Zj293g87p547eN1Ax2DLiPKQEn66tM7nFCXFLOebsY50Xg\n+yoFY+bdnMtzUwr7pkxKm17XGFe6HTCkBjq2gXUZAoGBAJ8KzbfnHGmmpimolnL+\nTPXNJP6QSwQi92UgIIbh8/BsFyW+5X/3cHhmFo5IlHlOktbT+zru6jv3EG6omqNQ\nWRlFVR6p6VwWjqzTZbR+dZUbRpSAk3jFZ+HIIkhfYpupMYySUf6UpQL10hQWrlYT\n87vTIKToXQtdQtTCFKSYkryf\n-----END PRIVATE KEY-----", "publicKey": "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoB0Da1HrUMRDrDX7A7fI\njDOP2EQJLwFyUDTeydNwmaweaoP+DRPcoSwPyMvlTF/uaQa8aF5+UnEzA7NINarU\nUeQ9rTi7E4ygvpu7Dr1c0YeLyVgFwJmlsX6woOfTNQJGL2yAx7Nw+YZ06htgDZgO\nQvZec2AnkER9zYmpmumXCPmmjpcBlpJVX0wjgZWyI4ahk69V2KPrxLvnewmUVAdQ\nEfAKYRFECisVc0An7ZykALepSGFo9CL7eeybWHirZr8jNWv5yJv+1R9FB3LWUJY7\nVSkoWBfzMdG4LoCUc2l8BXaChx3XT/nmTlJcZq2HTTOC5lPiqH52zMFKTV50auzl\npQIDAQAB\n-----END PUBLIC KEY-----", "algorithm": "RS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 99}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "RSA Cred"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "RSA Cred"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Verify" outputs:
      """
      [{"payload": {"user_id": 99}}]
      """

  Scenario: ES256 round trip using an EC PEM key pair credential
    Given the credential "EC Cred" of type "jwtAuth" with the data:
      """
      {"keyType": "pemKey", "privateKey": "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgx7nKNh+JlCIzdlk+\nG2Ihkd3/2qXwZ3zXUy0AyrH5ZIihRANCAAQINnIlHvhl1AUw66PHAoPZDpoyURPm\nKg6ANR2UNPblUg9OfrH1LFwvurSedmnpqfMLvZ9Y2l1O7GMRwAVw67S5\n-----END PRIVATE KEY-----", "publicKey": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAECDZyJR74ZdQFMOujxwKD2Q6aMlET\n5ioOgDUdlDT25VIPTn6x9SxcL7q0nnZp6anzC72fWNpdTuxjEcAFcOu0uQ==\n-----END PUBLIC KEY-----", "algorithm": "ES256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Verify | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 3}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "EC Cred"
    And the node "Verify" has parameters:
      """
      {"operation": "verify", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Verify" uses the "jwtAuth" credential "EC Cred"
    And the connections:
      """
      Start -> Sign
      Sign -> Verify
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Verify" outputs:
      """
      [{"payload": {"user_id": 3}}]
      """

  Scenario: Complete decode returns header, payload and signature
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Decode | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{\"user_id\": 1}", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the node "Decode" has parameters:
      """
      {"operation": "decode", "token": "={{ $json.token }}", "options": {"complete": true}}
      """
    And the node "Decode" uses the "jwtAuth" credential "HS Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Decode
      """
    When I execute the workflow
    Then the execution succeeds
    And the field "header.alg" of item 0 from the node "Decode" is "HS256"
    And the field "payload.user_id" of item 0 from the node "Decode" is 1
    And the field "signature" of item 0 from the node "Decode" is "$nonempty"

  Scenario: The claims builder writes literal field names, not the registered JWT claims
    Given the credential "Builder Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "builder-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Sign   | jwt           |
      | Decode | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": false, "claims": {"audience": "https://api.example.com", "issuer": "https://issuer.example.com", "subject": "user-7", "jwtid": "abc-123"}, "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "Builder Secret"
    And the node "Decode" has parameters:
      """
      {"operation": "decode", "token": "={{ $json.token }}", "options": {}}
      """
    And the node "Decode" uses the "jwtAuth" credential "Builder Secret"
    And the connections:
      """
      Start -> Sign
      Sign -> Decode
      """
    When I execute the workflow
    Then the execution succeeds
    And the field "payload.audience" of item 0 from the node "Decode" is "https://api.example.com"
    And the field "payload.issuer" of item 0 from the node "Decode" is "https://issuer.example.com"
    And the field "payload.subject" of item 0 from the node "Decode" is "user-7"
    And the field "payload.jwtid" of item 0 from the node "Decode" is "abc-123"
    And the field "payload.aud" of item 0 from the node "Decode" is null

  Scenario: Invalid JSON in the claims JSON payload fails with a clear error
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Sign  | jwt           |
    And the node "Sign" has parameters:
      """
      {"operation": "sign", "useJson": true, "claims": {}, "claimsJson": "{not valid json", "options": {}}
      """
    And the node "Sign" uses the "jwtAuth" credential "HS Secret"
    And the connections "Start -> Sign"
    When I execute the workflow
    Then the execution fails
    And the node "Sign" failed with an error containing "JSON"

  Scenario: Decoding without a token fails with a clear error
    Given the credential "HS Secret" of type "jwtAuth" with the data:
      """
      {"keyType": "passphrase", "secret": "top-secret", "algorithm": "HS256"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Jwt   | jwt           |
    And the node "Jwt" has parameters:
      """
      {"operation": "decode", "token": "", "options": {}}
      """
    And the node "Jwt" uses the "jwtAuth" credential "HS Secret"
    And the connections "Start -> Jwt"
    When I execute the workflow
    Then the execution fails
    And the node "Jwt" failed with an error containing "token was not provided"
