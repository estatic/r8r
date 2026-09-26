@spec-6.6 @phase-1 @node-transform
Feature: Data transformation nodes
  Sort, Limit, Aggregate, Split Out, Remove Duplicates and Summarize reshape
  item lists without code.

  Background:
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Node   | noOp          |
    And the connections "Start -> Node"
    And the trigger outputs the items:
      """
      [
        {"name": "Ada",   "age": 36, "region": "EU", "amount": 10, "email": "ada@x.io"},
        {"name": "Grace", "age": 45, "region": "US", "amount": 5,  "email": "grace@x.io"},
        {"name": "Linus", "age": 28, "region": "EU", "amount": 7,  "email": "ada@x.io"}
      ]
      """

  Scenario: Sort by a field, descending
    Given the node "Node" has the property "type" set to "n8n-nodes-base.sort"
    And the node "Node" has parameters:
      """
      {"sortFieldsUi": {"sortField": [{"fieldName": "age", "order": "descending"}]}, "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs items matching:
      """
      [{"name": "Grace"}, {"name": "Ada"}, {"name": "Linus"}]
      """

  Scenario: Limit keeps the first items
    Given the node "Node" has the property "type" set to "n8n-nodes-base.limit"
    And the node "Node" has parameters:
      """
      {"maxItems": 2}
      """
    When I execute the workflow
    Then the node "Node" outputs items matching:
      """
      [{"name": "Ada"}, {"name": "Grace"}]
      """

  Scenario: Limit can keep the last items
    Given the node "Node" has the property "type" set to "n8n-nodes-base.limit"
    And the node "Node" has parameters:
      """
      {"maxItems": 1, "keep": "lastItems"}
      """
    When I execute the workflow
    Then the node "Node" outputs items matching:
      """
      [{"name": "Linus"}]
      """

  Scenario: Aggregate individual fields into lists
    Given the node "Node" has the property "type" set to "n8n-nodes-base.aggregate"
    And the node "Node" has parameters:
      """
      {"fieldsToAggregate": {"fieldToAggregate": [{"fieldToAggregate": "name"}, {"fieldToAggregate": "age"}]}, "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs:
      """
      [{"name": ["Ada", "Grace", "Linus"], "age": [36, 45, 28]}]
      """

  Scenario: Aggregate all item data into one list
    Given the node "Node" has the property "type" set to "n8n-nodes-base.aggregate"
    And the node "Node" has parameters:
      """
      {"aggregate": "aggregateAllItemData", "destinationFieldName": "people", "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs 1 item
    And the field "people[2].name" of item 0 from the node "Node" is "Linus"

  Scenario: Split Out turns a list field into items
    Given the trigger outputs the items:
      """
      [{"order": "A", "lines": [{"sku": "x"}, {"sku": "y"}]}]
      """
    And the node "Node" has the property "type" set to "n8n-nodes-base.splitOut"
    And the node "Node" has parameters:
      """
      {"fieldToSplitOut": "lines", "include": "selectedOtherFields", "fieldsToInclude": "order", "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs:
      """
      [{"lines": {"sku": "x"}, "order": "A"}, {"lines": {"sku": "y"}, "order": "A"}]
      """

  Scenario: Remove duplicates compared on one field
    Given the node "Node" has the property "type" set to "n8n-nodes-base.removeDuplicates"
    And the node "Node" has the property "typeVersion" set to 2
    And the node "Node" has parameters:
      """
      {"operation": "removeDuplicateInputItems", "compare": "selectedFields", "fieldsToCompare": "email", "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs items matching:
      """
      [{"name": "Ada"}, {"name": "Grace"}]
      """

  Scenario: Remove duplicates of whole items
    Given the trigger outputs the items:
      """
      [{"a": 1}, {"a": 1}, {"a": 2}]
      """
    And the node "Node" has the property "type" set to "n8n-nodes-base.removeDuplicates"
    And the node "Node" has the property "typeVersion" set to 2
    And the node "Node" has parameters:
      """
      {"operation": "removeDuplicateInputItems", "compare": "allFields", "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs:
      """
      [{"a": 1}, {"a": 2}]
      """

  Scenario: Summarize sums a field split by another
    Given the node "Node" has the property "type" set to "n8n-nodes-base.summarize"
    And the node "Node" has the property "typeVersion" set to 1.1
    And the node "Node" has parameters:
      """
      {"fieldsToSummarize": {"values": [{"aggregation": "sum", "field": "amount"}, {"aggregation": "count", "field": "name"}]}, "fieldsToSplitBy": "region", "options": {}}
      """
    When I execute the workflow
    Then the node "Node" outputs:
      """
      [{"region": "EU", "sum_amount": 17, "count_name": 2}, {"region": "US", "sum_amount": 5, "count_name": 1}]
      """
