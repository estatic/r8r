@spec-6.6 @phase-4 @node-html
Feature: HTML node
  `n8n-nodes-base.html` (typeVersions 1, 1.1, 1.2): render an HTML template
  from expressions, extract content out of HTML with CSS selectors, and
  convert items into an HTML table.

  # ---- generateHtmlTemplate ------------------------------------------------

  Scenario: Generate HTML template renders expressions embedded in literal HTML
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "generateHtmlTemplate", "html": "<h1>{{ $json.title }}</h1><p>Price: {{ $json.price }}</p>"}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"title": "Widget", "price": 9.99}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"html": "<h1>Widget</h1><p>Price: 9.99</p>"}]
      """

  Scenario: Generate HTML template renders one item per input item
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "generateHtmlTemplate", "html": "<p>Hi {{ $json.name }}</p>"}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"name": "Ann"}, {"name": "Bo"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"html": "<p>Hi Ann</p>"}, {"html": "<p>Hi Bo</p>"}]
      """

  Scenario: Generate HTML template fails when an embedded expression throws
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "generateHtmlTemplate", "html": "<p>{{ (() => { throw new Error('boom') })() }}</p>"}
      """
    And the connections "Start -> Html"
    When I execute the workflow
    Then the execution fails
    And the node "Html" failed with an error containing "boom"

  # ---- extractHtmlContent ---------------------------------------------------

  Scenario: Extract HTML content returns the text of a matched element
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "price", "cssSelector": ".price", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<div class=\"price\">$9.99</div>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"price": "$9.99"}]
      """

  Scenario: Extract HTML content returns the inner HTML of a matched element
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "inner", "cssSelector": "#content", "returnValue": "html", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<div id=\"content\"><b>Hello</b> world</div>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"inner": "<b>Hello</b> world"}]
      """

  Scenario: Extract HTML content returns an attribute value
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "url", "cssSelector": "a", "returnValue": "attribute", "attribute": "href", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<a href=\"https://example.com\">link</a>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"url": "https://example.com"}]
      """

  Scenario: Extract HTML content returns a form element's value
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "val", "cssSelector": "input", "returnValue": "value", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<input type=\"text\" value=\"42\">"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"val": "42"}]
      """

  Scenario: Extract HTML content with Return Array collects every matched element separately
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "items", "cssSelector": "li", "returnValue": "text", "returnArray": true}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<ul><li>One</li><li>Two</li><li>Three</li></ul>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"items": ["One", "Two", "Three"]}]
      """

  Scenario: Extract HTML content trims and cleans up whitespace by default
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<p>  Hello   World  </p>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"text": "Hello World"}]
      """

  Scenario: Extract HTML content keeps raw whitespace when trimming and clean up are both disabled
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {"trimValues": false, "cleanUpText": false}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<p>  Hello   World  </p>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"text": "  Hello   World  "}]
      """

  Scenario: Extract HTML content reads HTML from binary data
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Make  | code          |
      | Html  | html          |
    And the node "Make" runs the JavaScript:
      """
      const data = Buffer.from('<p>Hello <b>World</b></p>').toString('base64');
      return [{ json: {}, binary: { file: { data, mimeType: 'text/html', fileName: 'page.html' } } }];
      """
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "sourceData": "binary", "dataPropertyName": "file", "extractionValues": {"values": [{"key": "greeting", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Make -> Html"
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"greeting": "Hello World"}]
      """

  Scenario: Extract HTML content (v1.2) resolves dataPropertyName as a dot path
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has the property "typeVersion" set to 1.2
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "payload.html", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"payload": {"html": "<p>Hi</p>"}}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"text": "Hi"}]
      """

  Scenario: Extract HTML content (v1) treats dataPropertyName as a literal key, not a dot path
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has the property "typeVersion" set to 1
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "payload.html", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"payload": {"html": "<p>Hi</p>"}}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Html" failed with an error containing "No property named \"payload.html\" exists"

  Scenario: Extract HTML content fails with a clear error for a missing data property
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "notThere", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<p>Hi</p>"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Html" failed with an error containing "No property named \"notThere\" exists"

  Scenario: Extract HTML content fails with a clear error for an invalid CSS selector
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "text", "cssSelector": ">> not a selector <<", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<p>Hi</p>"}]
      """
    When I execute the workflow
    Then the execution fails
    And the node "Html" failed with an error containing "is not valid"

  Scenario: Extract HTML content extracts one item per string when the source is an array of HTML documents
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "pages", "extractionValues": {"values": [{"key": "text", "cssSelector": "p", "returnValue": "text", "returnArray": false}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"pages": ["<p>A</p>", "<p>B</p>"]}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"text": "A"}, {"text": "B"}]
      """

  Scenario: Extract HTML content omits a key when its selector matches nothing
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "extractHtmlContent", "dataPropertyName": "page", "extractionValues": {"values": [{"key": "title", "cssSelector": "h1", "returnValue": "text", "returnArray": false}, {"key": "missing", "cssSelector": ".nope", "returnValue": "text", "returnArray": false}, {"key": "arr", "cssSelector": ".nope", "returnValue": "text", "returnArray": true}]}, "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"page": "<h1>Title</h1>"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"title": "Title", "arr": []}]
      """

  # ---- convertToHtmlTable ----------------------------------------------------

  Scenario: Convert to HTML table renders headers and rows from every item
    Given a workflow with nodes:
      | name  | type          | position |
      | Start | manualTrigger | 0,0      |
      | Make  | code          | 200,0    |
      | Html  | html          | 400,0    |
    And the node "Make" runs the JavaScript:
      """
      return [{ json: { id: 1, name: 'Ann' } }, { json: { id: 2, name: 'Bo' } }];
      """
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {}}
      """
    And the connections "Start -> Make -> Html"
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<table style='border-spacing:0; font-family:helvetica,arial,sans-serif' >"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<th>id</th><th>name</th>"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<td style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee' >1</td>"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<td style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee' >Ann</td>"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<tr  >"}]
      """

  Scenario: Convert to HTML table capitalizes headers when asked
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {"capitalize": true}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"first_name": "Ann"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<th>First Name</th>"}]
      """

  Scenario: Convert to HTML table drops the default styling with Custom Styling
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {"customStyling": true}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"a": 1}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<table  >"}]
      """

  Scenario: Convert to HTML table applies caption and every attributes option
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {"caption": "Report", "tableAttributes": "id=\"tbl\"", "headerAttributes": "class=\"hdr\"", "rowAttributes": "class=\"row\"", "cellAttributes": "class=\"cell\""}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"a": 1}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<caption>Report</caption>"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:id=\"tbl\">"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<thead style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee; text-align:left; color:#888; font-weight:normal' class=\"hdr\">"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<tr  class=\"row\">"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:class=\"cell\">1</td>"}]
      """

  Scenario: Convert to HTML table renders boolean fields as checkboxes
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"active": true}, {"active": false}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<input type=\"checkbox\" checked=\"checked\"/>"}]
      """
    And the node "Html" outputs:
      """
      [{"table": "$contains:<input type=\"checkbox\" />"}]
      """

  Scenario: Convert to HTML table renders a missing field as the literal text "undefined"
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      [{"name": "Ann"}, {"other": "x"}]
      """
    When I execute the workflow
    Then the node "Html" outputs:
      """
      [{"table": "$contains:<td style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee' >undefined</td>"}]
      """

  Scenario: Convert to HTML table with no input items produces no output and does not crash
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Html  | html          |
    And the node "Html" has parameters:
      """
      {"operation": "convertToHtmlTable", "options": {}}
      """
    And the connections "Start -> Html"
    And the trigger outputs the items:
      """
      []
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Html" was not executed
