@spec-6.6 @phase-5
Feature: OpenAI node
  The standalone "OpenAI" action node (`@n8n/n8n-nodes-langchain.openAi`,
  not the chat-model sub-node), faithful to n8n's `nodes/vendors/OpenAi`
  v2 (typeVersion 2.3, the editor's default) and v1 (`text:message`) code.
  The OpenAI API is a mock HTTP service.

  Background:
    Given a mock HTTP service
    And the credential "Mock OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-test-123", "url": "%{MOCK_URL}/v1"}
      """

  # ---- text:response (v2 default) ------------------------------------------

  Scenario: Messaging a model with the Responses API
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {
        "id": "resp_1",
        "output": [
          {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Paris", "annotations": []}]}
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5}
      }
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "What is the capital of France?"}]},
        "simplify": true,
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Paris", "annotations": []}]}]}]
      """
    And the last request to "/v1/responses" had a JSON body matching:
      """
      {"model": "gpt-4o-mini", "input": [{"role": "user", "content": [{"type": "input_text", "text": "What is the capital of France?"}]}]}
      """
    And the last request to "/v1/responses" had the header "authorization" equal to "Bearer sk-test-123"

  Scenario: Messaging a model with simplify off returns the raw response
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {"id": "resp_2", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Paris", "annotations": []}]}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "simplify": false,
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"id": "resp_2", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Paris", "annotations": []}]}]}]
      """

  Scenario: Messaging a model with JSON output mode
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {"id": "resp_3", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "{\"city\": \"Paris\"}", "annotations": []}]}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Give me the capital of France as JSON"}]},
        "simplify": true,
        "options": {"textFormat": {"textOptions": {"type": "json_object"}}}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": {"city": "Paris"}, "annotations": []}]}]}]
      """
    And the last request to "/v1/responses" had a JSON body matching:
      """
      {"text": {"format": {"type": "json_object"}}, "input": [{"role": "system"}, {"role": "user"}]}
      """

  Scenario: Messaging a model with options (temperature, max tokens, instructions)
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {"id": "resp_4", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok", "annotations": []}]}]}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {"temperature": 0.2, "maxTokens": 128, "topP": 0.9, "instructions": "Be terse."}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/responses" had a JSON body matching:
      """
      {"temperature": 0.2, "max_output_tokens": 128, "top_p": 0.9, "instructions": "Be terse."}
      """

  Scenario: An empty prompt fails with a clear error
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": ""}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "A non-empty prompt is required."

  # ---- text:message (v1) ----------------------------------------------------

  Scenario: Messaging a model with the Chat Completions API (v1)
    Given the mock service responds to POST "/v1/chat/completions" with status 200 and body:
      """
      {
        "id": "chatcmpl-1",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "Paris"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
      }
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Ask   | lc.openAi     | 1.8         |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "message",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "messages": {"values": [{"content": "What is the capital of France?", "role": "user"}]},
        "simplify": true,
        "jsonOutput": false,
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"index": 0, "message": {"role": "assistant", "content": "Paris"}, "finish_reason": "stop"}]
      """
    And the last request to "/v1/chat/completions" had a JSON body matching:
      """
      {"model": "gpt-4o-mini", "messages": [{"content": "What is the capital of France?", "role": "user"}]}
      """

  Scenario: Messaging a model (v1) with JSON output mode
    Given the mock service responds to POST "/v1/chat/completions" with status 200 and body:
      """
      {
        "id": "chatcmpl-2",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "{\"city\": \"Paris\"}"}, "finish_reason": "stop"}]
      }
      """
    And a workflow with nodes:
      | name  | type          | typeVersion |
      | Start | manualTrigger |             |
      | Ask   | lc.openAi     | 1.8         |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "message",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "messages": {"values": [{"content": "Give me the capital of France as JSON", "role": "user"}]},
        "simplify": true,
        "jsonOutput": true,
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"index": 0, "message": {"role": "assistant", "content": {"city": "Paris"}}, "finish_reason": "stop"}]
      """
    And the last request to "/v1/chat/completions" had a JSON body matching:
      """
      {"response_format": {"type": "json_object"}, "messages": [{"role": "system", "content": "You are a helpful assistant designed to output JSON."}, {"content": "Give me the capital of France as JSON", "role": "user"}]}
      """

  # ---- text:classify ---------------------------------------------------------

  Scenario: Classifying text for violations
    Given the mock service responds to POST "/v1/moderations" with status 200 and body:
      """
      {"id": "modr-1", "model": "omni-moderation-latest", "results": [{"flagged": true, "categories": {"violence": true}}]}
      """
    And a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Classify | lc.openAi     |
    And the node "Classify" has parameters:
      """
      {"resource": "text", "operation": "classify", "input": "I will hurt you", "simplify": false, "options": {}}
      """
    And the node "Classify" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Classify"
    When I execute the workflow
    Then the execution succeeds
    And the node "Classify" outputs:
      """
      [{"flagged": true, "categories": {"violence": true}}]
      """
    And the last request to "/v1/moderations" had a JSON body matching:
      """
      {"input": "I will hurt you", "model": "omni-moderation-latest"}
      """

  Scenario: Classifying text with simplify returns only the flag
    Given the mock service responds to POST "/v1/moderations" with status 200 and body:
      """
      {"results": [{"flagged": false, "categories": {}}]}
      """
    And a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Classify | lc.openAi     |
    And the node "Classify" has parameters:
      """
      {"resource": "text", "operation": "classify", "input": "Hello there", "simplify": true, "options": {}}
      """
    And the node "Classify" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Classify"
    When I execute the workflow
    Then the execution succeeds
    And the node "Classify" outputs:
      """
      [{"flagged": false}]
      """

  # ---- image:generate ---------------------------------------------------------

  Scenario: Generating an image and returning a URL
    Given the mock service responds to POST "/v1/images/generations" with status 200 and body:
      """
      {"data": [{"url": "https://example.com/cat.png"}]}
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | Generate  | lc.openAi     |
    And the node "Generate" has parameters:
      """
      {
        "resource": "image",
        "operation": "generate",
        "modelId": {"__rl": true, "value": "dall-e-3", "mode": "list"},
        "prompt": "A cute cat eating a dinosaur",
        "options": {"returnImageUrls": true}
      }
      """
    And the node "Generate" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Generate"
    When I execute the workflow
    Then the execution succeeds
    And the node "Generate" outputs:
      """
      [{"url": "https://example.com/cat.png"}]
      """
    And the last request to "/v1/images/generations" had a JSON body matching:
      """
      {"prompt": "A cute cat eating a dinosaur", "model": "dall-e-3", "response_format": "url"}
      """

  Scenario: Generating an image and returning binary data
    Given the mock service responds to POST "/v1/images/generations" with status 200 and body:
      """
      {"data": [{"b64_json": "aGVsbG8gd29ybGQ="}]}
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | Generate  | lc.openAi     |
    And the node "Generate" has parameters:
      """
      {
        "resource": "image",
        "operation": "generate",
        "modelId": {"__rl": true, "value": "dall-e-3", "mode": "list"},
        "prompt": "A cute cat eating a dinosaur",
        "options": {"returnImageUrls": false, "binaryPropertyOutput": "image"}
      }
      """
    And the node "Generate" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Generate"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Generate": [
                {
                  "data": {
                    "main": [[
                      {
                        "binary": {"image": {"data": "aGVsbG8gd29ybGQ=", "mimeType": "image/png"}}
                      }
                    ]]
                  }
                }
              ]
            }
          }
        }
      }
      """
    And the last request to "/v1/images/generations" had a JSON body matching:
      """
      {"response_format": "b64_json"}
      """

  # ---- image:analyze -----------------------------------------------------------

  Scenario: Analyzing an image from a URL
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A cat."}]}]}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Analyze | lc.openAi     |
    And the node "Analyze" has parameters:
      """
      {
        "resource": "image",
        "operation": "analyze",
        "modelId": {"__rl": true, "value": "gpt-4o", "mode": "list"},
        "text": "What's in this image?",
        "inputType": "url",
        "imageUrls": "https://example.com/cat.jpg",
        "simplify": true,
        "options": {}
      }
      """
    And the node "Analyze" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Analyze"
    When I execute the workflow
    Then the execution succeeds
    And the node "Analyze" outputs:
      """
      [{"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A cat."}]}]}]
      """
    And the last request to "/v1/responses" had a JSON body matching:
      """
      {"model": "gpt-4o", "input": [{"role": "user", "content": [{"type": "input_text", "text": "What's in this image?"}, {"type": "input_image", "detail": "auto", "image_url": "https://example.com/cat.jpg"}]}]}
      """

  Scenario: Analyzing an image from binary input
    Given the mock service responds to POST "/v1/responses" with status 200 and body:
      """
      {"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A cat."}]}]}
      """
    And a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Analyze | lc.openAi     |
    And the node "Analyze" has parameters:
      """
      {
        "resource": "image",
        "operation": "analyze",
        "modelId": {"__rl": true, "value": "gpt-4o", "mode": "list"},
        "text": "What's in this image?",
        "inputType": "base64",
        "binaryPropertyName": "data",
        "simplify": true,
        "options": {}
      }
      """
    And the node "Analyze" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Analyze"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "pretend image bytes" and mime type "image/jpeg"
    When I execute the workflow
    Then the execution succeeds
    And the last request to "/v1/responses" had a JSON body matching:
      """
      {"input": [{"role": "user", "content": [{"type": "input_text", "text": "What's in this image?"}, {"type": "input_image", "detail": "auto"}]}]}
      """

  # ---- audio:generate (TTS) ----------------------------------------------------

  Scenario: Generating speech from text returns binary audio
    Given the mock service responds to POST "/v1/audio/speech" with status 200 and body:
      """
      hello audio bytes
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | TTS   | lc.openAi     |
    And the node "TTS" has parameters:
      """
      {"resource": "audio", "operation": "generate", "model": "tts-1", "input": "Hello there", "voice": "alloy", "options": {}}
      """
    And the node "TTS" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> TTS"
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "TTS": [
                {
                  "data": {
                    "main": [[
                      {
                        "binary": {"data": {"data": "aGVsbG8gYXVkaW8gYnl0ZXM=", "mimeType": "audio/mpeg", "fileName": "audio.mp3"}}
                      }
                    ]]
                  }
                }
              ]
            }
          }
        }
      }
      """
    And the last request to "/v1/audio/speech" had a JSON body matching:
      """
      {"model": "tts-1", "input": "Hello there", "voice": "alloy", "response_format": "mp3"}
      """

  # ---- audio:transcribe / translate --------------------------------------------

  Scenario: Transcribing an audio file
    Given the mock service responds to POST "/v1/audio/transcriptions" with status 200 and body:
      """
      {"text": "Hello there"}
      """
    And a workflow with nodes:
      | name        | type          |
      | Start       | manualTrigger |
      | Transcribe  | lc.openAi     |
    And the node "Transcribe" has parameters:
      """
      {"resource": "audio", "operation": "transcribe", "binaryPropertyName": "data", "options": {"language": "en"}}
      """
    And the node "Transcribe" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Transcribe"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "pretend audio bytes" and mime type "audio/mpeg"
    When I execute the workflow
    Then the execution succeeds
    And the node "Transcribe" outputs:
      """
      [{"text": "Hello there"}]
      """
    And the last request to "/v1/audio/transcriptions" had the multipart field "model" equal to "whisper-1"
    And the last request to "/v1/audio/transcriptions" had the multipart field "language" equal to "en"
    And the last request to "/v1/audio/transcriptions" had a multipart file field "file" with filename "data.mpeg"

  Scenario: Translating an audio file
    Given the mock service responds to POST "/v1/audio/translations" with status 200 and body:
      """
      {"text": "Hello there"}
      """
    And a workflow with nodes:
      | name      | type          |
      | Start     | manualTrigger |
      | Translate | lc.openAi     |
    And the node "Translate" has parameters:
      """
      {"resource": "audio", "operation": "translate", "binaryPropertyName": "data", "options": {}}
      """
    And the node "Translate" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Translate"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "pretend audio bytes" and mime type "audio/mpeg"
    When I execute the workflow
    Then the execution succeeds
    And the node "Translate" outputs:
      """
      [{"text": "Hello there"}]
      """
    And the last request to "/v1/audio/translations" had the multipart field "model" equal to "whisper-1"
    And the last request to "/v1/audio/translations" had no multipart field "language"

  # ---- file:upload / list / deleteFile -----------------------------------------

  Scenario: Uploading a file from binary data
    Given the mock service responds to POST "/v1/files" with status 200 and body:
      """
      {"id": "file-abc123", "object": "file", "bytes": 140, "filename": "data.txt", "purpose": "user_data"}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Upload | lc.openAi     |
    And the node "Upload" has parameters:
      """
      {"resource": "file", "operation": "upload", "binaryPropertyName": "data", "options": {}}
      """
    And the node "Upload" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Upload"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "hello file" and mime type "text/plain"
    When I execute the workflow
    Then the execution succeeds
    And the node "Upload" outputs:
      """
      [{"id": "file-abc123", "object": "file", "bytes": 140, "filename": "data.txt", "purpose": "user_data"}]
      """
    And the last request to "/v1/files" had the multipart field "purpose" equal to "user_data"
    And the last request to "/v1/files" had a multipart file field "file" with filename "data.txt"

  Scenario: Listing files
    Given the mock service responds to GET "/v1/files" with status 200 and body:
      """
      {"data": [{"id": "file-1", "filename": "a.txt"}, {"id": "file-2", "filename": "b.txt"}]}
      """
    And a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | List | lc.openAi     |
    And the node "List" has parameters:
      """
      {"resource": "file", "operation": "list", "options": {}}
      """
    And the node "List" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> List"
    When I execute the workflow
    Then the execution succeeds
    And the node "List" outputs:
      """
      [{"id": "file-1", "filename": "a.txt"}, {"id": "file-2", "filename": "b.txt"}]
      """

  Scenario: Deleting a file
    Given the mock service responds to DELETE "/v1/files/file-abc123" with status 200 and body:
      """
      {"id": "file-abc123", "object": "file", "deleted": true}
      """
    And a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Delete | lc.openAi     |
    And the node "Delete" has parameters:
      """
      {"resource": "file", "operation": "deleteFile", "fileId": {"__rl": true, "value": "file-abc123", "mode": "id"}}
      """
    And the node "Delete" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Delete"
    When I execute the workflow
    Then the execution succeeds
    And the node "Delete" outputs:
      """
      [{"id": "file-abc123", "object": "file", "deleted": true}]
      """

  # ---- errors, auth, continueOnFail, custom base URL, and unsupported ops -----

  Scenario: A 400 from OpenAI maps to n8n's bad-request message
    Given the mock service responds to POST "/v1/responses" with status 400 and body:
      """
      {"error": {"message": "Invalid value for 'model'", "type": "invalid_request_error", "code": "model_not_found"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "bogus-model", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Bad request - please check your parameters"

  Scenario: A 401 from OpenAI maps to n8n's authorization-failed message
    Given the mock service responds to POST "/v1/responses" with status 401 and body:
      """
      {"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "Authorization failed - please check your credentials"

  Scenario: A 429 from OpenAI maps to n8n's rate-limit message
    Given the mock service responds to POST "/v1/responses" with status 429 and body:
      """
      {"error": {"message": "Rate limit reached", "type": "requests", "code": "rate_limit_exceeded"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "OpenAI: Rate limit reached"

  Scenario: A missing OpenAI credential fails with a clear message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "does not have any credentials"

  Scenario: An OpenAI credential with a blank API key fails with a clear message
    Given the credential "Blank" of type "openAiApi" with the data:
      """
      {"apiKey": "", "url": "%{MOCK_URL}/v1"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Blank"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "OpenAI credentials are not set"

  Scenario: The API key never appears in the execution data
    Given the mock service responds to POST "/v1/responses" with status 401 and body:
      """
      {"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution data does not contain "sk-test-123"

  Scenario: continueOnFail turns an OpenAI error into an error item
    Given the mock service responds to POST "/v1/responses" with status 400 and body:
      """
      {"error": {"message": "Invalid value for 'model'", "type": "invalid_request_error"}}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "bogus-model", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" has properties:
      """
      {"onError": "continueRegularOutput"}
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the node "Ask" outputs:
      """
      [{"error": "Bad request - please check your parameters"}]
      """

  Scenario: A custom base URL is used for requests
    Given the mock service responds to POST "/custom/v1/responses" with status 200 and body:
      """
      {"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok", "annotations": []}]}]}
      """
    And the credential "Custom OpenAI" of type "openAiApi" with the data:
      """
      {"apiKey": "sk-custom", "url": "%{MOCK_URL}/custom/v1"}
      """
    And a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {
        "resource": "text",
        "operation": "response",
        "modelId": {"__rl": true, "value": "gpt-4o-mini", "mode": "list"},
        "responses": {"values": [{"type": "text", "role": "user", "content": "Hi"}]},
        "options": {}
      }
      """
    And the node "Ask" uses the "openAiApi" credential "Custom OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution succeeds
    And the mock service received 1 requests to "/custom/v1/responses"

  Scenario: An unimplemented resource returns a clear "not supported" message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {"resource": "assistant", "operation": "message"}
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"

  Scenario: An unimplemented operation returns a clear "not supported" message
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Ask   | lc.openAi     |
    And the node "Ask" has parameters:
      """
      {"resource": "video", "operation": "generate"}
      """
    And the node "Ask" uses the "openAiApi" credential "Mock OpenAI"
    And the connections "Start -> Ask"
    When I execute the workflow
    Then the execution fails
    And the execution error message contains "not supported natively yet"
