@spec-2.4 @spec-6.2 @phase-1
Feature: Execution order v1 (depth-first) and legacy v0 (breadth-first)
  `settings.executionOrder` picks the scheduler. v1, the default, finishes
  one branch before starting the next, taking branches in canvas order (top
  to bottom). v0 runs nodes level by level. Both must be supported.

  Background:
    Given a workflow with nodes:
      | name   | type          | position |
      | Start  | manualTrigger | 0,0      |
      | Top    | noOp          | 200,-100 |
      | Top 2  | noOp          | 400,-100 |
      | Bottom | noOp          | 200,100  |
      | Bottom 2 | noOp        | 400,100  |
    And the connections:
      """
      Start -> Bottom -> Bottom 2
      Start -> Top -> Top 2
      """

  Scenario: v1 runs each branch to the end, the upper branch first
    Given the workflow setting "executionOrder" is "v1"
    When I execute the workflow
    Then the execution succeeds
    And the nodes ran in the order "Start, Top, Top 2, Bottom, Bottom 2"

  Scenario: v0 runs the workflow level by level
    Given the workflow setting "executionOrder" is "v0"
    When I execute the workflow
    Then the execution succeeds
    And the nodes ran in the order "Start, Bottom, Top, Bottom 2, Top 2"

  # Checked against n8n 2.35: a missing setting means the legacy order, so
  # old workflows keep their behaviour. New workflows are saved with "v1".
  Scenario: A workflow without executionOrder runs as v0
    Given the workflow has no "executionOrder" setting
    When I execute the workflow
    Then the nodes ran in the order "Start, Bottom, Top, Bottom 2, Top 2"

  Scenario: Canvas position, not connection order, decides which branch goes first
    Given I edit the workflow "BDD workflow"
    And the node "Top" has the property "position" set to [200, 300]
    And the node "Top 2" has the property "position" set to [400, 300]
    When I execute the workflow
    Then the nodes ran in the order "Start, Bottom, Bottom 2, Top, Top 2"
