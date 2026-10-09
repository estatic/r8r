import { describe, expect, it } from 'vitest'
import { buildHttp, loadHttp } from './httpRequest'

describe('HTTP Request form', () => {
  it('starts as a GET with nothing else', () => {
    expect(loadHttp({})).toEqual({ method: 'GET', url: '', query: [], headers: [], bodyType: 'none', body: '' })
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

  it('takes a URL that is an expression', () => {
    expect(buildHttp(loadHttp({ url: '{{ $json.link }}' }))).toEqual({ fields: { method: 'GET', url: '{{ $json.link }}' } })
  })
})
