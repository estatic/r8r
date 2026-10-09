/** The Webhook trigger's form, and its stored parameters. */

export interface WebhookForm {
  method: string
  path: string
  /** `run` is the whole run record (nodes saved before response modes). */
  respond: 'immediately' | 'lastNode' | 'run'
  responseData: 'firstEntryJson' | 'allEntries' | 'noData'
  responseCode: string
}

export const WEBHOOK_METHODS = ['ANY', 'GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD']
export const WEBHOOK_KEYS = ['method', 'path', 'respond', 'response_data', 'response_code']

export function loadWebhook(p: Record<string, unknown>): WebhookForm {
  const isNew = Object.keys(p).filter((k) => k !== 'auth').length === 0
  const respond = p.respond === 'immediately' || p.respond === 'lastNode' ? p.respond : isNew ? 'lastNode' : 'run'
  const data = p.response_data === 'allEntries' || p.response_data === 'noData' ? p.response_data : 'firstEntryJson'
  return {
    method: typeof p.method === 'string' ? p.method.toUpperCase() : 'POST',
    path: typeof p.path === 'string' ? p.path : '',
    respond,
    responseData: data,
    responseCode: typeof p.response_code === 'number' ? String(p.response_code) : p.respond === 'immediately' ? '202' : '200',
  }
}

export function buildWebhook(f: WebhookForm): { fields: Record<string, unknown> } | { error: string } {
  const path = f.path.trim().replace(/^\/+|\/+$/g, '')
  if (!path) return { error: 'Enter the webhook path, e.g. orders or users/:id.' }
  if (!/^[A-Za-z0-9._~:\-/]+$/.test(path)) return { error: 'The path may use letters, digits, - _ . ~ / and :name parts.' }
  const fields: Record<string, unknown> = { method: f.method, path }
  if (f.respond === 'run') return { fields }
  const code = Number(f.responseCode)
  if (!Number.isInteger(code) || code < 100 || code > 599) return { error: 'The response code must be an HTTP status from 100 to 599.' }
  fields.respond = f.respond
  fields.response_code = code
  if (f.respond === 'lastNode') fields.response_data = f.responseData
  return { fields }
}
