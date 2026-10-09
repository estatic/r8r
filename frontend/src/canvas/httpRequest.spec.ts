import { describe, expect, it } from 'vitest'
import { buildHttp, loadHttp } from './httpRequest'

describe('HTTP Request form', () => {
  it('starts as a GET with nothing else', () => {
    expect(loadHttp({})).toEqual({ method: 'GET', url: '', query: [], headers: [], bodyType: 'none', body: '', formFields: [], timeoutMs: '', responseFormat: 'auto' })
  })

  it('round-trips a request with query, headers and a JSON body', () => {
    const fields = {
      method: 'POST',
      url: 'https://api.example.com/items',
      query: { page: '{{ $json.page }}' },
      headers: { 'X-Trace': 'abc' },
      body: { name: '{{ $json.name }}', count: 2 },
    }
    expect(buildHttp(loadHttp(fields))).toEqual({ fields })
  })

  it('skips blank rows and says what is wrong', () => {
    expect(buildHttp({ ...loadHttp({ url: 'https://x' }), query: [{ name: '', value: '' }] })).toEqual({ fields: { method: 'GET', url: 'https://x' } })
    expect(buildHttp(loadHttp({}))).toEqual({ error: 'Enter the URL to call.' })
    expect(buildHttp(loadHttp({ url: 'example.com' }))).toEqual({ error: 'The URL must start with http:// or https://.' })
    expect(buildHttp({ ...loadHttp({ url: 'https://x' }), headers: [{ name: '', value: 'v' }] })).toEqual({ error: 'Every header needs a name.' })
    expect(buildHttp({ ...loadHttp({ url: 'https://x' }), bodyType: 'json', body: '{oops' })).toHaveProperty('error')
  })

  it('round-trips a form body, a text body and the options', () => {
    const form = { method: 'POST', url: 'https://x.example', body_type: 'form', body: { name: '{{ $json.name }}' }, timeout_ms: 5000, response_format: 'text' }
    expect(buildHttp(loadHttp(form))).toEqual({ fields: form })
    const text = { method: 'PUT', url: 'https://x.example', body_type: 'text', body: 'hello {{ $json.name }}', response_format: 'json' }
    expect(buildHttp(loadHttp(text))).toEqual({ fields: text })
    expect(buildHttp({ ...loadHttp({ url: 'https://x' }), timeoutMs: '0' })).toHaveProperty('error')
  })

  it('takes a URL that is an expression', () => {
    expect(buildHttp(loadHttp({ url: '{{ $json.link }}' }))).toEqual({ fields: { method: 'GET', url: '{{ $json.link }}' } })
  })
})
