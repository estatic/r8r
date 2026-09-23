export type NodeCategory = 'trigger' | 'action' | 'flowControl' | 'ai'

export interface NodeTypeMeta {
  type_name: string
  display_name: string
  icon: string
  category: NodeCategory
  description: string
  credential_types: string[]
  output_ports: string[]
}

export interface RetryPolicy {
  max_tries: number
  wait_ms: number
}

export interface NodeSettings {
  retry: RetryPolicy | null
  timeout_ms: number | null
  continue_on_fail: boolean
}

export interface NodeInstance {
  id: string
  node_type: string
  position: [number, number]
  parameters: Record<string, unknown>
  disabled: boolean
  settings?: NodeSettings
}

export interface Connection {
  from_node: string
  from_output: number
  to_node: string
  to_input: number
  error: boolean
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

export type CredentialFieldType = 'text' | 'password'

export interface CredentialField {
  name: string
  label: string
  field_type: CredentialFieldType
  required: boolean
}

export interface CredentialTypeSchema {
  credential_type: string
  display_name: string
  generic: boolean
  fields: CredentialField[]
}
