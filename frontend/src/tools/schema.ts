import type { Tool, ToolArgumentType } from '../types/domain'

export interface ToolArgument {
  name: string
  type: ToolArgumentType
  description: string
  required: boolean
}

export const TOOL_NODE_TYPES: { value: string; label: string }[] = [
  { value: 'core.httpRequest', label: 'HTTP Request' },
  { value: 'telegram.sendMessage', label: 'Telegram: Send Message' },
  { value: 'core.code', label: 'Code' },
]

/** Starter parameters per node type, showing where {{ $args.x }} goes. */
export const PARAMETER_TEMPLATES: Record<string, Record<string, unknown>> = {
  'core.httpRequest': { method: 'GET', url: 'https://api.example.com/search?q={{ $args.query }}' },
  'telegram.sendMessage': { chat_id: '', text: '{{ $args.message }}' },
  'core.code': { script: 'return [{ json: { result: $args } }]' },
}

const HTTP_AUTH_TYPE: Record<string, string> = { bearerToken: 'bearer', apiKeyHeader: 'apiKey', basicAuth: 'basic' }

export function authForCredentialType(nodeType: string, credentialId: string, credentialType: string | undefined): Record<string, string> {
  const type = nodeType === 'core.httpRequest' && credentialType ? HTTP_AUTH_TYPE[credentialType] : undefined
  return type ? { type, credential_id: credentialId } : { credential_id: credentialId }
}

export function argsToSchema(args: ToolArgument[]): Tool['argument_schema'] {
  return {
    type: 'object',
    properties: Object.fromEntries(
      args.map((a) => [a.name, a.description ? { type: a.type, description: a.description } : { type: a.type }]),
    ),
    required: args.filter((a) => a.required).map((a) => a.name),
  }
}

export function schemaToArgs(schema: Tool['argument_schema']): ToolArgument[] {
  const required = schema.required ?? []
  return Object.entries(schema.properties ?? {}).map(([name, p]) => ({
    name,
    type: p.type,
    description: p.description ?? '',
    required: required.includes(name),
  }))
}