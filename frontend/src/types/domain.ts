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

export type ExecutionStatus = 'Running' | 'Success' | 'Error' | 'Canceled'
export type ExecutionMode = 'Manual' | 'Webhook' | 'Schedule' | 'Telegram'

export interface Execution {
  id: string
  workflow_id: string
  status: ExecutionStatus
  mode: ExecutionMode
  node_outputs: Record<string, Item[]>
  /** How each node that ran ended, and the items per output ("0", "1", ..., "error"). */
  node_runs?: Record<string, NodeRun>
  started_at: string
  finished_at: string | null
}

/** "running" only exists client-side, between node_started and its end. */
export type NodeRunStatus = 'running' | 'success' | 'error' | 'skipped'

export interface NodeRun {
  status: NodeRunStatus
  counts: Record<string, number>
}

export interface CredentialSummary {
  id: string
  name: string
  credential_type: string
  owner_id: string
  created_at: string
  updated_at: string
  used_by: number
}

export interface CredentialDetail extends CredentialSummary {
  fields: Record<string, string>
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

export type ToolArgumentType = 'string' | 'number' | 'integer' | 'boolean'

export interface Tool {
  id: string
  name: string
  description: string
  node_type: string
  argument_schema: { type: 'object'; properties?: Record<string, { type: ToolArgumentType; description?: string }>; required?: string[] }
  parameters: Record<string, unknown>
  created_at: string
  updated_at: string
  used_by?: number
}

/** The AI Agent settings form's fields (stored in the node's parameters). */
export interface AgentFields {
  provider: string
  model: string
  system_prompt: string
  user_message: string
  max_iterations: number | string
  tool_ids: string[]
  /** Chat memory: remember the last `memory_window` exchanges per session. */
  memory_enabled: boolean
  memory_window: number | string
  /** Empty: each Telegram chat is its own conversation. */
  memory_session_key: string
  /** Whether the node already had `memory` (so turning it off keeps it, off). */
  memory_stored: boolean
}
