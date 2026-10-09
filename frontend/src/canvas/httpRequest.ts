/** The HTTP Request node's form, and its stored parameters. */

export interface Pair {
  name: string
  value: string
}

export interface HttpForm {
  method: string
  url: string
  query: Pair[]
  headers: Pair[]
  /** none, or a JSON body (values may be {{ expressions }}). */
  bodyType: 'none' | 'json'
  body: string
}

export const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS']
export const HTTP_KEYS = ['method', 'url', 'query', 'headers', 'body']

const str = (v: unknown) => (v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v))
const pairs = (v: unknown): Pair[] =>
  v && typeof v === 'object' && !Array.isArray(v) ? Object.entries(v as Record<string, unknown>).map(([name, value]) => ({ name, value: str(value) })) : []

export function loadHttp(p: Record<string, unknown>): HttpForm {
  const method = typeof p.method === 'string' ? p.method.toUpperCase() : 'GET'
  const hasBody = p.body !== undefined && p.body !== null
  return {
    method,
    url: str(p.url),
    query: pairs(p.query),
    headers: pairs(p.headers),
    bodyType: hasBody ? 'json' : 'none',
    body: hasBody ? JSON.stringify(p.body, null, 2) : '',
  }
}

function toObject(rows: Pair[], what: string): { value: Record<string, string> } | { error: string } {
  const out: Record<string, string> = {}
  for (const r of rows) {
    const name = r.name.trim()
    if (!name && !r.value.trim()) continue
    if (!name) return { error: `Every ${what} needs a name.` }
    if (name in out) return { error: `The ${what} "${name}" is set twice.` }
    out[name] = r.value
  }
  return { value: out }
}

/** The node's parameters, or why the form can't be saved. */
export function buildHttp(f: HttpForm): { fields: Record<string, unknown> } | { error: string } {
  const url = f.url.trim()
  if (!url) return { error: 'Enter the URL to call.' }
  if (!url.includes('{{') && !/^https?:\/\//i.test(url)) return { error: 'The URL must start with http:// or https://.' }
  const fields: Record<string, unknown> = { method: f.method, url }
  const query = toObject(f.query, 'query parameter')
  if ('error' in query) return query
  if (Object.keys(query.value).length > 0) fields.query = query.value
  const headers = toObject(f.headers, 'header')
  if ('error' in headers) return headers
  if (Object.keys(headers.value).length > 0) fields.headers = headers.value
  if (f.bodyType === 'json' && f.body.trim()) {
    try {
      fields.body = JSON.parse(f.body)
    } catch {
      return { error: 'The body must be valid JSON (put expressions inside strings: "{{ $json.id }}").' }
    }
  }
  return { fields }
}
