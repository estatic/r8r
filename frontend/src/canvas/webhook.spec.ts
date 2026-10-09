import { describe, expect, it } from 'vitest'
import { buildWebhook, loadWebhook } from './webhook'

describe('Webhook form', () => {
  it('starts a new node as a POST answered with the last node\'s data', () => {
    expect(loadWebhook({})).toEqual({ method: 'POST', path: '', respond: 'lastNode', responseData: 'firstEntryJson', responseCode: '200' })
  })

  it('round-trips each response mode', () => {
    for (const fields of [
      { method: 'PUT', path: 'users/:id', respond: 'lastNode', response_code: 201, response_data: 'allEntries' },
      { method: 'ANY', path: 'ping', respond: 'immediately', response_code: 200 },
      { method: 'POST', path: 'old-hook' },
    ]) {
      expect(buildWebhook(loadWebhook(fields))).toEqual({ fields })
    }
  })

  it('keeps 202 for an older "immediately" node and trims the path', () => {
    expect(buildWebhook({ ...loadWebhook({ path: 'x', respond: 'immediately' }), path: '/x/' })).toEqual({ fields: { method: 'POST', path: 'x', respond: 'immediately', response_code: 202 } })
  })

  it('says what is wrong', () => {
    expect(buildWebhook(loadWebhook({}))).toHaveProperty('error')
    expect(buildWebhook({ ...loadWebhook({}), path: 'a b' })).toHaveProperty('error')
    expect(buildWebhook({ ...loadWebhook({}), path: 'a', responseCode: '999' })).toEqual({ error: 'The response code must be an HTTP status from 100 to 599.' })
  })
})
