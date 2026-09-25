import { describe, it, expect } from 'vitest'
import { agentSetupProblems, describeSetupProblems } from './setup'
import type { NodeInstance } from '../types/domain'

const node = (id: string, node_type: string, parameters: Record<string, unknown>): NodeInstance => ({
  id,
  node_type,
  position: [0, 0],
  parameters,
  disabled: false,
})

describe('agentSetupProblems', () => {
  it('flags an AI Agent missing model and user message', () => {
    expect(agentSetupProblems([node('agent', 'ai.agent', { auth: { credential_id: 'c' } })])).toEqual([
      { nodeId: 'agent', missing: ['Model', 'User message'] },
    ])
  })

  it('treats whitespace-only values as missing', () => {
    expect(agentSetupProblems([node('a', 'ai.agent', { model: '  ', user_message: 'hi' })])).toEqual([{ nodeId: 'a', missing: ['Model'] }])
  })

  it('accepts a configured agent and ignores other node types', () => {
    expect(
      agentSetupProblems([node('a', 'ai.agent', { model: 'qwen', user_message: 'hi' }), node('s', 'core.set', {})]),
    ).toEqual([])
  })

  it('ignores a disabled agent, which never runs', () => {
    expect(agentSetupProblems([{ ...node('a', 'ai.agent', {}), disabled: true }])).toEqual([])
  })
})

describe('describeSetupProblems', () => {
  it('names each node and what it needs', () => {
    expect(describeSetupProblems([{ nodeId: 'agent', missing: ['Model', 'User message'] }])).toBe(
      'AI Agent "agent" needs Model and User message. Open the node to set them.',
    )
    expect(
      describeSetupProblems([
        { nodeId: 'a', missing: ['Model'] },
        { nodeId: 'b', missing: ['User message'] },
      ]),
    ).toBe('AI Agent "a" needs Model. AI Agent "b" needs User message. Open the node to set them.')
  })
})
