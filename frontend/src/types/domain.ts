export interface NodeInstance {
  id: string
  node_type: string
  position: [number, number]
  parameters: Record<string, unknown>
  disabled: boolean
}

export interface Connection {
  from_node: string
  from_output: number
  to_node: string
  to_input: number
}

export interface Workflow {
  id: string
  name: string
  active: boolean
  nodes: NodeInstance[]
  connections: Connection[]
  created_at: string
  updated_at: string
}

export interface Item {
  json: unknown
  binary: unknown
}

export type ExecutionStatus = 'Running' | 'Success' | 'Error'
export type ExecutionMode = 'Manual' | 'Webhook' | 'Schedule' | 'Telegram'

export interface Execution {
  id: string
  workflow_id: string
  status: ExecutionStatus
  mode: ExecutionMode
  node_outputs: Record<string, Item[]>
  started_at: string
  finished_at: string | null
}

export interface CredentialSummary {
  id: string
  name: string
  credential_type: string
  owner_id: string
  created_at: string
  updated_at: string
}
