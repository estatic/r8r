import type { Connection, NodeInstance, Workflow } from '../types/domain'

function same(a: Connection, b: Connection): boolean {
  return (
    a.from_node === b.from_node &&
    a.from_output === b.from_output &&
    a.to_node === b.to_node &&
    a.to_input === b.to_input &&
    a.error === b.error
  )
}

/** Deletes a node and every link to or from it. */
export function removeNode(workflow: Workflow, nodeId: string): void {
  workflow.nodes = workflow.nodes.filter((n) => n.id !== nodeId)
  workflow.connections = workflow.connections.filter((c) => c.from_node !== nodeId && c.to_node !== nodeId)
}

/** Deletes one link. */
export function removeConnection(workflow: Workflow, connection: Connection): void {
  workflow.connections = workflow.connections.filter((c) => !same(c, connection))
}

/**
 * Puts `node` into the link `connection` (A -> B becomes A -> node -> B),
 * halfway between A and B. The first half keeps A's output (or its error
 * route); the second leaves the node's first output into B's input.
 */
export function insertNodeIntoConnection(workflow: Workflow, connection: Connection, node: NodeInstance): void {
  const from = workflow.nodes.find((n) => n.id === connection.from_node)
  const to = workflow.nodes.find((n) => n.id === connection.to_node)
  if (from && to) {
    node.position = [(from.position[0] + to.position[0]) / 2, (from.position[1] + to.position[1]) / 2]
  }
  const index = workflow.connections.findIndex((c) => same(c, connection))
  const halves: Connection[] = [
    { ...connection, to_node: node.id, to_input: 0 },
    { from_node: node.id, from_output: 0, to_node: connection.to_node, to_input: connection.to_input, error: false },
  ]
  workflow.nodes.push(node)
  if (index === -1) workflow.connections.push(...halves)
  else workflow.connections.splice(index, 1, ...halves)
}
