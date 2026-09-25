@spec-2.4 @spec-6.4 @phase-1
Feature: Expression data proxy
  Expressions see the current item and the run through n8n's variables:
  `$json`, `$binary`, `$input`, `$('Node')`, `$node`, `$prevNode`,
  `$runIndex`, `$itemIndex`, `$workflow`, `$execution`, `$now`, `$today`,
  `$env` (blocked by default) and `$jmespath`.

  Scenario Outline: $json and $input read the current input
    Given the input items:
      """
      [{"name": "Ada", "tags": ["x", "y"], "nested": {"deep": {"value": 42}}}, {"name": "Grace"}]
      """
    When I evaluate the expression "<expression>"
    Then the result is <result>

    Examples:
      | expression                               | result  |
      | ={{ $json.name }}                        | "Ada"   |
      | ={{ $json['name'] }}                     | "Ada"   |
      | ={{ $json.tags.length }}                 | 2       |
      | ={{ $json.nested.deep.value }}           | 42      |
      | ={{ $input.item.json.name }}             | "Ada"   |
      | ={{ $input.first().json.name }}          | "Ada"   |
      | ={{ $input.last().json.name }}           | "Grace" |
      | ={{ $input.all().length }}               | 2       |
      | ={{ $input.all().map(i => i.json.name) }} | ["Ada", "Grace"] |

  Scenario: $itemIndex is the index of the item being processed
    Given the input items:
      """
      [{"v": "a"}, {"v": "b"}, {"v": "c"}]
      """
    When I evaluate the expression "={{ $itemIndex }}"
    Then the results for each item are [0, 1, 2]

  Scenario: $json is evaluated per item
    Given the input items:
      """
      [{"v": 1}, {"v": 2}, {"v": 3}]
      """
    When I evaluate the expression "={{ $json.v * 10 }}"
    Then the results for each item are [10, 20, 30]

  Scenario: $('Node') reaches earlier nodes, following paired items
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Customer | set           |
      | Lookup   | set           |
      | Report   | set           |
    And the node "Customer" sets the fields:
      """
      {"id": "={{ $json.id }}", "name": "={{ $json.name }}"}
      """
    And the node "Lookup" sets the fields:
      """
      {"score": "={{ $json.id * 100 }}"}
      """
    And the node "Report" sets the fields:
      """
      {
        "name": "={{ $('Customer').item.json.name }}",
        "score": "={{ $json.score }}",
        "firstCustomer": "={{ $('Customer').first().json.name }}",
        "customers": "={{ $('Customer').all().length }}",
        "legacy": "={{ $node['Customer'].json.name }}"
      }
      """
    And the connections "Start -> Customer -> Lookup -> Report"
    And the trigger outputs the items:
      """
      [{"id": 1, "name": "Ada"}, {"id": 2, "name": "Grace"}]
      """
    When I execute the workflow
    Then the execution succeeds
    And the node "Report" outputs:
      """
      [
        {"name": "Ada", "score": 100, "firstCustomer": "Ada", "customers": 2, "legacy": "Ada"},
        {"name": "Grace", "score": 200, "firstCustomer": "Ada", "customers": 2, "legacy": "Ada"}
      ]
      """

  Scenario: $('Node').item follows paired items through a fan-out
    Given a workflow with nodes:
      | name   | type          | parameters                                                             |
      | Start  | manualTrigger |                                                                        |
      | Order  | set           |                                                                        |
      | Lines  | splitOut      | {"fieldToSplitOut": "lines", "include": "noOtherFields", "options": {}} |
      | Label  | set           |                                                                        |
    And the node "Order" adds the fields:
      """
      {"orderId": "={{ $json.orderId }}"}
      """
    And the node "Label" sets the fields:
      """
      {"label": "={{ $('Order').item.json.orderId }}-{{ $json.sku }}"}
      """
    And the connections "Start -> Order -> Lines -> Label"
    And the trigger outputs the items:
      """
      [{"orderId": "A", "lines": [{"sku": "x"}, {"sku": "y"}]}, {"orderId": "B", "lines": [{"sku": "z"}]}]
      """
    When I execute the workflow
    Then the node "Label" outputs:
      """
      [{"label": "A-x"}, {"label": "A-y"}, {"label": "B-z"}]
      """

  Scenario: $('Node') on a node that did not run is an error
    Given a workflow with nodes:
      | name   | type          |
      | Start  | manualTrigger |
      | Never  | noOp          |
      | Read   | set           |
    And the node "Read" sets the fields:
      """
      {"x": "={{ $('Never').item.json.x }}"}
      """
    And the connections "Start -> Read"
    When I execute the workflow
    Then the execution fails
    And the node "Read" failed with an error containing "Never"

  Scenario: $prevNode names the node the input came from
    Given a workflow with nodes:
      | name     | type          |
      | Start    | manualTrigger |
      | Upstream | noOp          |
      | Probe    | set           |
    And the node "Probe" sets the fields:
      """
      {"previous": "={{ $prevNode.name }}", "output": "={{ $prevNode.outputIndex }}"}
      """
    And the connections "Start -> Upstream -> Probe"
    When I execute the workflow
    Then the node "Probe" outputs:
      """
      [{"previous": "Upstream", "output": 0}]
      """

  Scenario: $workflow describes the running workflow
    Given a workflow named "Invoice sync" with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Probe | set           |
    And the node "Probe" sets the fields:
      """
      {"name": "={{ $workflow.name }}", "active": "={{ $workflow.active }}"}
      """
    And the connections "Start -> Probe"
    When I execute the workflow
    Then the node "Probe" outputs:
      """
      [{"name": "Invoice sync", "active": false}]
      """

  Scenario: $execution exposes the id and the resume URL
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Probe | set           |
    And the node "Probe" sets the fields:
      """
      {"id": "={{ $execution.id }}", "resumeUrl": "={{ $execution.resumeUrl }}"}
      """
    And the connections "Start -> Probe"
    When I execute the workflow
    Then the node "Probe" outputs:
      """
      [{"id": "$nonempty", "resumeUrl": "$contains:/webhook-waiting/"}]
      """

  Scenario: $execution.customData stores searchable metadata
    Given a workflow with nodes:
      | name  | type          |
      | Start | manualTrigger |
      | Tag   | code          |
    And the node "Tag" runs the JavaScript:
      """
      $execution.customData.set('customer', 'ada');
      return [{ json: { customer: $execution.customData.get('customer') } }];
      """
    And the connections "Start -> Tag"
    When I execute the workflow
    Then the node "Tag" outputs:
      """
      [{"customer": "ada"}]
      """

  Scenario: $env is blocked by default
    Given the environment variable "BDD_SECRET" is "do-not-leak"
    When I evaluate the expression "={{ $env.BDD_SECRET }}"
    Then the expression fails with an error containing "access to env vars denied"

  Scenario: $env is readable when N8N_BLOCK_ENV_ACCESS_IN_NODE is false
    Given the environment variable "BDD_SECRET" is "visible"
    And the environment variable "N8N_BLOCK_ENV_ACCESS_IN_NODE" is "false"
    When I evaluate the expression "={{ $env.BDD_SECRET }}"
    Then the result is "visible"

  Scenario: $jmespath queries JSON
    Given the input item:
      """
      {"people": [{"name": "Ada", "age": 36}, {"name": "Grace", "age": 45}]}
      """
    When I evaluate the expression "={{ $jmespath($json, 'people[?age > `40`].name') }}"
    Then the result is ["Grace"]

  @phase-2
  Scenario: $vars reads instance variables
    Given a running r8r server with an owner and an API key
    And I send a POST request to "/api/v1/variables" with body:
      """
      {"key": "REGION", "value": "eu-west-1"}
      """
    And a workflow named "Vars" with nodes:
      | name    | type    | parameters                                                                         |
      | Webhook | webhook | {"httpMethod": "GET", "path": "vars", "responseMode": "lastNode", "options": {}}   |
      | Probe   | set     |                                                                                    |
    And the node "Probe" sets the fields:
      """
      {"region": "={{ $vars.REGION }}"}
      """
    And the connections "Webhook -> Probe"
    And the workflow is active
    When I send a GET request to "/webhook/vars"
    Then the response JSON is:
      """
      {"region": "eu-west-1"}
      """
