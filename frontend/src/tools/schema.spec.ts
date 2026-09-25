import { describe, it, expect } from 'vitest'
import { argsToSchema, schemaToArgs, authForCredentialType, PARAMETER_TEMPLATES } from './schema'

describe('tool schema helpers', () => {
  it('round-trips arguments through a JSON schema', () => {
    const args = [
      { name: 'city', type: 'string' as const, description: 'City name', required: true },
      { name: 'days', type: 'integer' as const, description: '', required: false },
    ]
    const schema = argsToSchema(args)
    expect(schema).toEqual({
      type: 'object',
      properties: { city: { type: 'string', description: 'City name' }, days: { type: 'integer' } },
      required: ['city'],
    })
    expect(schemaToArgs(schema)).toEqual(args)
  })

  it('offers an $args template per tool node type', () => {
    expect(JSON.stringify(PARAMETER_TEMPLATES['core.httpRequest'])).toContain('{{ $args.query }}')
    expect(JSON.stringify(PARAMETER_TEMPLATES['telegram.sendMessage'])).toContain('{{ $args.message }}')
    expect(JSON.stringify(PARAMETER_TEMPLATES['core.code'])).toContain('$args')
  })

  it('maps a generic credential type to the HTTP auth type', () => {
    expect(authForCredentialType('core.httpRequest', 'c1', 'bearerToken')).toEqual({ type: 'bearer', credential_id: 'c1' })
    expect(authForCredentialType('core.httpRequest', 'c1', 'apiKeyHeader')).toEqual({ type: 'apiKey', credential_id: 'c1' })
    expect(authForCredentialType('core.httpRequest', 'c1', 'basicAuth')).toEqual({ type: 'basic', credential_id: 'c1' })
    expect(authForCredentialType('telegram.sendMessage', 'c2', 'telegramApi')).toEqual({ credential_id: 'c2' })
  })
})