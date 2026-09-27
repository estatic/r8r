@spec-6.6 @phase-4 @node-files
Feature: Extract from File / Convert to File nodes
  `n8n-nodes-base.extractFromFile` (typeVersion 1) turns binary data into
  JSON items; `n8n-nodes-base.convertToFile` (typeVersions 1, 1.1) turns
  JSON items into binary data. Implemented operations: csv, xlsx, fromJson,
  text and html for extraction; csv, xlsx, toJson, toText, html and toBinary
  for conversion. Everything else (ics, ods, pdf, rtf, xls for extraction;
  iCal, ods, rtf, xls for conversion) is expected to fail with a clear
  "not supported natively yet" error rather than doing the wrong thing.

  # ---- CSV ------------------------------------------------------------

  Scenario: Items to CSV to items round-trips headers, unicode and comma-quoted fields
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | ToCSV   | convertToFile  |
      | FromCSV | extractFromFile |
    And the connections "Start -> ToCSV", "ToCSV -> FromCSV"
    And the trigger outputs the items:
      """
      [{"name": "Adaäü", "note": "hello, world"}, {"name": "Bob", "note": "plain"}]
      """
    And the node "ToCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {}}
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromCSV" outputs:
      """
      [{"name": "Adaäü", "note": "hello, world"}, {"name": "Bob", "note": "plain"}]
      """

  Scenario: CSV round-trip honours a custom delimiter
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | ToCSV   | convertToFile  |
      | FromCSV | extractFromFile |
    And the connections "Start -> ToCSV", "ToCSV -> FromCSV"
    And the trigger outputs the items:
      """
      [{"a": "1,2", "b": "x"}]
      """
    And the node "ToCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {"delimiter": ";"}}
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {"delimiter": ";"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromCSV" outputs:
      """
      [{"a": "1,2", "b": "x"}]
      """

  Scenario: Extract from CSV with header row off returns array-shaped rows
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromCSV | extractFromFile |
    And the connections "Start -> FromCSV"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with mime type "text/csv" and content:
      """
      1,2,3
      4,5,6
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {"headerRow": false}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromCSV" outputs:
      """
      [{"row": ["1", "2", "3"]}, {"row": ["4", "5", "6"]}]
      """

  Scenario: Extract from CSV honours fromLine and maxRowCount
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromCSV | extractFromFile |
    And the connections "Start -> FromCSV"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with mime type "text/csv" and content:
      """
      junk,junk
      h1,h2
      a,1
      b,2
      c,3
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {"fromLine": 1, "maxRowCount": 2}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromCSV" outputs:
      """
      [{"h1": "a", "h2": "1"}, {"h1": "b", "h2": "2"}]
      """

  Scenario: Malformed CSV (a row with the wrong number of fields) fails with a clear error
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromCSV | extractFromFile |
    And the connections "Start -> FromCSV"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with mime type "text/csv" and content:
      """
      a,b
      1,2,3
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "FromCSV" failed with an error containing "not in csv format"

  Scenario: Each extracted CSV row is paired to its source item
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromCSV | extractFromFile |
    And the connections "Start -> FromCSV"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with mime type "text/csv" and content:
      """
      a,b
      1,2
      3,4
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the items of the node "FromCSV" are paired as:
      | item | pairedItem   |
      | 0    | {"item": 0}  |
      | 1    | {"item": 0}  |

  # ---- XLSX -------------------------------------------------------------

  Scenario: Items to XLSX to items round-trips string, number and boolean fields
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | ToXLSX   | convertToFile  |
      | FromXLSX | extractFromFile |
    And the connections "Start -> ToXLSX", "ToXLSX -> FromXLSX"
    And the trigger outputs the items:
      """
      [{"name": "Ada", "age": 36, "active": true}, {"name": "Bob", "age": 29, "active": false}]
      """
    And the node "ToXLSX" has parameters:
      """
      {"operation": "xlsx", "binaryPropertyName": "data", "options": {}}
      """
    And the node "FromXLSX" has parameters:
      """
      {"operation": "xlsx", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromXLSX" outputs items matching:
      """
      [{"name": "Ada", "age": 36}, {"name": "Bob", "age": 29}]
      """

  Scenario: Malformed XLSX data fails with a clear error
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | FromXLSX | extractFromFile |
    And the connections "Start -> FromXLSX"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "not an xlsx file" and mime type "application/octet-stream"
    And the node "FromXLSX" has parameters:
      """
      {"operation": "xlsx", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "FromXLSX" failed with an error containing "not in xlsx format"

  # ---- JSON ---------------------------------------------------------------

  Scenario: Convert to File (toJson, mode once) combines every item into one array file
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | ToJSON   | convertToFile  |
      | FromJSON | extractFromFile |
    And the connections "Start -> ToJSON", "ToJSON -> FromJSON"
    And the trigger outputs the items:
      """
      [{"a": 1}, {"a": 2}]
      """
    And the node "ToJSON" has parameters:
      """
      {"operation": "toJson", "mode": "once", "binaryPropertyName": "data", "options": {}}
      """
    And the node "FromJSON" has parameters:
      """
      {"operation": "fromJson", "binaryPropertyName": "data", "destinationKey": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "ToJSON" outputs 1 item
    And the items of the node "ToJSON" are paired as:
      | item | pairedItem                            |
      | 0    | [{"item": 0}, {"item": 1}]            |
    And the node "FromJSON" outputs:
      """
      [{"data": [{"a": 1}, {"a": 2}]}]
      """

  Scenario: Convert to File (toJson, mode each) writes one file per item
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | ToJSON   | convertToFile  |
      | FromJSON | extractFromFile |
    And the connections "Start -> ToJSON", "ToJSON -> FromJSON"
    And the trigger outputs the items:
      """
      [{"a": 1}, {"a": 2}]
      """
    And the node "ToJSON" has parameters:
      """
      {"operation": "toJson", "mode": "each", "binaryPropertyName": "data", "options": {}}
      """
    And the node "FromJSON" has parameters:
      """
      {"operation": "fromJson", "binaryPropertyName": "data", "destinationKey": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "ToJSON" outputs 2 items
    And the node "FromJSON" outputs:
      """
      [{"data": {"a": 1}}, {"data": {"a": 2}}]
      """

  # ---- text -----------------------------------------------------------

  Scenario: Extract from a text file puts its decoded content under destinationKey
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | FromText | extractFromFile |
    And the connections "Start -> FromText"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "Hello, World!" and mime type "text/plain"
    And the node "FromText" has parameters:
      """
      {"operation": "text", "binaryPropertyName": "data", "destinationKey": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromText" outputs:
      """
      [{"data": "Hello, World!"}]
      """

  Scenario: Convert to File (toText) writes a JSON field to a text file with correct binary metadata
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | ToText | convertToFile |
    And the connections "Start -> ToText"
    And the trigger outputs the items:
      """
      [{"note": "hello"}]
      """
    And the node "ToText" has parameters:
      """
      {"operation": "toText", "sourceProperty": "note", "binaryPropertyName": "data", "options": {"fileName": "note.txt"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "ToText": [
                {"data": {"main": [[
                  {"binary": {"data": {"data": "aGVsbG8=", "mimeType": "text/plain", "fileExtension": "txt", "fileName": "note.txt"}}}
                ]]}}
              ]
            }
          }
        }
      }
      """

  Scenario: Convert to File (toBinary) decodes a base64 field into arbitrary binary data
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | ToBinary | convertToFile |
    And the connections "Start -> ToBinary"
    And the trigger outputs the items:
      """
      [{"payload": "aGVsbG8="}]
      """
    And the node "ToBinary" has parameters:
      """
      {"operation": "toBinary", "sourceProperty": "payload", "binaryPropertyName": "data", "options": {"mimeType": "application/octet-stream", "fileName": "blob.bin"}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "ToBinary": [
                {"data": {"main": [[
                  {"binary": {"data": {"data": "aGVsbG8=", "mimeType": "application/octet-stream", "fileExtension": "bin", "fileName": "blob.bin"}}}
                ]]}}
              ]
            }
          }
        }
      }
      """

  # ---- HTML -----------------------------------------------------------

  Scenario: Items to HTML table to items round-trips headers and cell values
    Given a workflow with nodes:
      | name     | type           |
      | Start    | manualTrigger  |
      | ToHTML   | convertToFile  |
      | FromHTML | extractFromFile |
    And the connections "Start -> ToHTML", "ToHTML -> FromHTML"
    And the trigger outputs the items:
      """
      [{"name": "Ada", "role": "Engineer"}, {"name": "Bob", "role": "Manager"}]
      """
    And the node "ToHTML" has parameters:
      """
      {"operation": "html", "binaryPropertyName": "data", "options": {}}
      """
    And the node "FromHTML" has parameters:
      """
      {"operation": "html", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "FromHTML" outputs:
      """
      [{"name": "Ada", "role": "Engineer"}, {"name": "Bob", "role": "Manager"}]
      """

  # ---- errors -----------------------------------------------------------

  Scenario: Extract from File with a missing binary property fails with a clear error
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromCSV | extractFromFile |
    And the connections "Start -> FromCSV"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "FromCSV" has parameters:
      """
      {"operation": "csv", "binaryPropertyName": "nope", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "FromCSV" failed with an error containing "binary file 'nope'"

  Scenario: Extract from File rejects an operation that is not supported natively
    Given a workflow with nodes:
      | name    | type           |
      | Start   | manualTrigger  |
      | FromPDF | extractFromFile |
    And the connections "Start -> FromPDF"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "data" with content "%PDF-1.4" and mime type "application/pdf"
    And the node "FromPDF" has parameters:
      """
      {"operation": "pdf", "binaryPropertyName": "data", "options": {}}
      """
    When I execute the workflow
    Then the execution fails
    And the node "FromPDF" failed with an error containing "not supported natively yet"

  Scenario: Convert to File rejects an operation that is not supported natively
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | ToICal | convertToFile |
    And the connections "Start -> ToICal"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "ToICal" has parameters:
      """
      {"operation": "iCal"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "ToICal" failed with an error containing "not supported natively yet"
