@spec-6.6 @phase-4 @node-compression
Feature: Compression node (v1, v1.1)
  Compresses binary properties into a zip or gzip archive, and decompresses
  zip/gzip archives back into one binary property per contained file. Tar
  and tar.gz, which real n8n also supports, are out of scope here.

  Scenario: Compressing two files into a zip and decompressing round-trips their content and names
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Zip   | compression   |
      | Unzip | compression   |
    And the connections "Start -> Zip", "Zip -> Unzip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "file1" with content "hello world" and mime type "text/plain"
    And the trigger item 0 has the binary property "file2" with content "second file" and mime type "text/plain"
    And the node "Zip" has parameters:
      """
      {"operation": "compress", "binaryPropertyName": "file1,file2", "outputFormat": "zip", "fileName": "bundle.zip", "binaryPropertyOutput": "data"}
      """
    And the node "Unzip" has parameters:
      """
      {"operation": "decompress", "binaryPropertyName": "data", "outputPrefix": "file_"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Zip": [
                {"data": {"main": [[{"binary": {"data": {"fileName": "bundle.zip", "mimeType": "application/zip", "fileExtension": "zip"}}}]]}}
              ],
              "Unzip": [
                {
                  "data": {
                    "main": [[
                      {
                        "binary": {
                          "file_0": {"data": "aGVsbG8gd29ybGQ=", "fileName": "file1.txt", "mimeType": "text/plain", "fileExtension": "txt"},
                          "file_1": {"data": "c2Vjb25kIGZpbGU=", "fileName": "file2.txt", "mimeType": "text/plain", "fileExtension": "txt"}
                        }
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

  Scenario: Compressing one file with gzip and decompressing round-trips its content
    Given a workflow with nodes:
      | name    | type          |
      | Start   | manualTrigger |
      | Gzip    | compression   |
      | Gunzip  | compression   |
    And the connections "Start -> Gzip", "Gzip -> Gunzip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "report" with content "csv,data" and mime type "text/csv"
    And the node "Gzip" has parameters:
      """
      {"operation": "compress", "binaryPropertyName": "report", "outputFormat": "gzip", "binaryPropertyOutput": "data"}
      """
    And the node "Gunzip" has parameters:
      """
      {"operation": "decompress", "binaryPropertyName": "data", "outputPrefix": "file_"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Gzip": [
                {"data": {"main": [[{"binary": {"data": {"fileName": "report.csv.gz", "mimeType": "application/gzip", "fileExtension": "gz"}}}]]}}
              ],
              "Gunzip": [
                {
                  "data": {
                    "main": [[
                      {"binary": {"file_0": {"data": "Y3N2LGRhdGE=", "fileName": "report.csv", "mimeType": "text/csv", "fileExtension": "csv"}}}
                    ]]
                  }
                }
              ]
            }
          }
        }
      }
      """

  Scenario: Compressing multiple binary properties with gzip produces one output property per file
    Given a workflow with nodes:
      | name | type          |
      | Start| manualTrigger |
      | Gzip | compression   |
    And the connections "Start -> Gzip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "a" with content "hello world" and mime type "text/plain"
    And the trigger item 0 has the binary property "b" with content "second file" and mime type "text/plain"
    And the trigger item 0 has the binary property "c" with content "csv,data" and mime type "text/csv"
    And the node "Gzip" has parameters:
      """
      {"operation": "compress", "binaryPropertyName": "a,b,c", "outputFormat": "gzip", "binaryPropertyOutput": "data"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "Gzip": [
                {
                  "data": {
                    "main": [[
                      {
                        "binary": {
                          "data": {"fileName": "a.txt.gz", "mimeType": "application/gzip", "fileExtension": "gz"},
                          "data1": {"fileName": "b.txt.gz", "mimeType": "application/gzip", "fileExtension": "gz"},
                          "data2": {"fileName": "c.csv.gz", "mimeType": "application/gzip", "fileExtension": "gz"}
                        }
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

  Scenario: Compressing with a missing binary property fails with a clear error
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Zip   | compression   |
    And the connections "Start -> Zip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the node "Zip" has parameters:
      """
      {"operation": "compress", "binaryPropertyName": "missingProp", "outputFormat": "zip", "fileName": "out.zip"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Zip" failed with an error containing "Item has no binary field 'missingProp'"

  Scenario: Decompressing a corrupt zip archive fails with a clear error
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Unzip | compression   |
    And the connections "Start -> Unzip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "archive" with content "this is not a real zip file" and mime type "application/zip"
    And the node "Unzip" has parameters:
      """
      {"operation": "decompress", "binaryPropertyName": "archive", "outputPrefix": "file_"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Unzip" failed with an error containing "is not a valid zip archive"

  Scenario: Decompressing a non-archive binary fails with a clear error
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Unzip | compression   |
    And the connections "Start -> Unzip"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "doc" with content "plain text file" and mime type "text/plain"
    And the node "Unzip" has parameters:
      """
      {"operation": "decompress", "binaryPropertyName": "doc", "outputPrefix": "file_"}
      """
    When I execute the workflow
    Then the execution fails
    And the node "Unzip" failed with an error containing "Unsupported archive format"

  Scenario: The output prefix controls the names of decompressed files
    Given a workflow with nodes:
      | name        | type          |
      | Start       | manualTrigger |
      | Zip         | compression   |
      | UnzipCustom | compression   |
    And the connections "Start -> Zip", "Zip -> UnzipCustom"
    And the trigger outputs the items:
      """
      [{}]
      """
    And the trigger item 0 has the binary property "file1" with content "hello world" and mime type "text/plain"
    And the trigger item 0 has the binary property "file2" with content "second file" and mime type "text/plain"
    And the node "Zip" has parameters:
      """
      {"operation": "compress", "binaryPropertyName": "file1,file2", "outputFormat": "zip", "fileName": "bundle.zip", "binaryPropertyOutput": "data"}
      """
    And the node "UnzipCustom" has parameters:
      """
      {"operation": "decompress", "binaryPropertyName": "data", "outputPrefix": "out_"}
      """
    When I execute the workflow
    Then the execution succeeds
    And the execution result matches:
      """
      {
        "data": {
          "resultData": {
            "runData": {
              "UnzipCustom": [
                {
                  "data": {
                    "main": [[
                      {
                        "binary": {
                          "out_0": {"data": "aGVsbG8gd29ybGQ=", "fileName": "file1.txt"},
                          "out_1": {"data": "c2Vjb25kIGZpbGU=", "fileName": "file2.txt"}
                        }
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
