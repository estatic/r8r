import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import NodeConfigPanel from './NodeConfigPanel.vue'
import { useCredentialsStore } from '../stores/credentials'
import type { NodeInstance, NodeSettings } from '../types/domain'

const node: NodeInstance = {
  id: 'n1',
  node_type: 'core.set',
  position: [0, 0],
  parameters: { foo: 'bar' },
  disabled: false,
}

describe('NodeConfigPanel', () => {
  it('emits update with the parsed parameters on Apply', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{"foo":"baz"}')
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
    const events = wrapper.emitted('update')
    expect(events).toBeTruthy()
    expect((events![0][0] as NodeInstance).parameters).toEqual({ foo: 'baz' })
  })

  it('shows an error and does not emit update on invalid JSON', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{not valid json')
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
    expect(wrapper.text()).toContain('must be valid JSON')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  async function clickApply(wrapper: ReturnType<typeof mount>) {
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
  }

  describe('Telegram Trigger "Trigger on"', () => {
    const tg = (parameters: Record<string, unknown>): NodeInstance => ({ ...node, node_type: 'telegram.trigger', parameters })
    const box = (w: ReturnType<typeof mount>, value: string) => w.find(`[data-testid="tg-update-${value}"]`)

    it('starts on "All updates" when nothing is chosen, and keeps it on Apply', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: tg({}) } })
      expect((box(wrapper, '*').element as HTMLInputElement).checked).toBe(true)
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.updates).toEqual(['*'])
    })

    it('saves the chosen update types', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: tg({ updates: ['*'] }) } })
      await box(wrapper, 'message').setValue(true) // choosing one leaves "All updates"
      await box(wrapper, 'callback_query').setValue(true)
      expect((box(wrapper, '*').element as HTMLInputElement).checked).toBe(false)
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.updates).toEqual(['message', 'callback_query'])
    })

    it('loads a saved choice', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: tg({ updates: ['poll', 'pre_checkout_query'] }) } })
      expect((box(wrapper, 'poll').element as HTMLInputElement).checked).toBe(true)
      expect((box(wrapper, 'pre_checkout_query').element as HTMLInputElement).checked).toBe(true)
      expect((box(wrapper, 'message').element as HTMLInputElement).checked).toBe(false)
    })

    it('refuses an empty choice', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: tg({ updates: ['message'] }) } })
      await box(wrapper, 'message').setValue(false)
      await clickApply(wrapper)
      expect(wrapper.text()).toContain('Choose at least one update type')
      expect(wrapper.emitted('update')).toBeFalsy()
    })
  })

  describe('Set node fields', () => {
    const setNode = (fields: Record<string, unknown>): NodeInstance => ({ ...node, node_type: 'core.set', parameters: { fields } })
    const sources = [
      { nodeId: 'agent', direct: true, fields: [{ path: '.response', preview: '"hi"', segments: ['response'], expression: '{{ $json.response }}' }, { path: '.message.chat.id', preview: '42', segments: ['message', 'chat', 'id'], expression: '{{ $json.message.chat.id }}' }] },
      { nodeId: 'tg', direct: false, fields: [{ path: '.update_id', preview: '9', segments: ['update_id'], expression: '{{ $node["tg"].json.update_id }}' }] },
    ]
    const props = (fields: Record<string, unknown>) => ({ node: setNode(fields), inputSources: sources, nodeLabels: { agent: '🤖 AI Agent', tg: '📨 Telegram Trigger' } })
    const applied = (w: ReturnType<typeof mount>) => (w.emitted('update')![0][0] as NodeInstance).parameters.fields

    it('shows the stored fields and saves them unchanged', async () => {
      const wrapper = mount(NodeConfigPanel, { props: props({ chat_id: '{{ $json.message.chat.id }}', n: 3 }) })
      expect(wrapper.findAll('[data-testid="set-field-row"]')).toHaveLength(2)
      await clickApply(wrapper)
      expect(applied(wrapper)).toEqual({ chat_id: '{{ $json.message.chat.id }}', n: 3 })
    })

    it('adds a field picked from the data reaching the node, naming it after the pick', async () => {
      const wrapper = mount(NodeConfigPanel, { props: props({}) })
      await wrapper.find('[data-testid="add-field"]').trigger('click')
      await wrapper.find('[data-testid="pick-field"]').trigger('click')
      const picker = wrapper.find('[data-testid="field-picker"]')
      expect(picker.text()).toContain('🤖 AI Agent — input')
      expect(picker.text()).toContain('📨 Telegram Trigger')
      await picker.findAll('button').find((b) => b.text().startsWith('message.chat.id'))!.trigger('click')
      expect((wrapper.find('[data-testid="field-name"]').element as HTMLInputElement).value).toBe('id')
      await wrapper.find('[data-testid="field-name"]').setValue('chat_id')
      await clickApply(wrapper)
      expect(applied(wrapper)).toEqual({ chat_id: '{{ $json.message.chat.id }}' })
    })

    it('adds a fixed, typed value and removes a field', async () => {
      const wrapper = mount(NodeConfigPanel, { props: props({ old: 'x' }) })
      await wrapper.find('[data-testid="remove-field"]').trigger('click')
      await wrapper.find('[data-testid="add-field"]').trigger('click')
      await wrapper.find('[data-testid="field-name"]').setValue('limit')
      await wrapper.find('[data-testid="mode-fixed"]').trigger('click')
      await wrapper.find('[data-testid="field-type"]').setValue('number')
      await wrapper.find('[data-testid="field-value"]').setValue('5')
      await clickApply(wrapper)
      expect(applied(wrapper)).toEqual({ limit: 5 })
    })

    it('explains how to get data to pick from before a run', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: setNode({}), inputSources: [{ nodeId: 'tg', direct: true, fields: [] }], nodeLabels: {} } })
      await wrapper.find('[data-testid="add-field"]').trigger('click')
      await wrapper.find('[data-testid="pick-field"]').trigger('click')
      expect(wrapper.find('[data-testid="field-picker"]').text()).toContain('Run the workflow once')
    })

    it('refuses a blank field name', async () => {
      const wrapper = mount(NodeConfigPanel, { props: props({}) })
      await wrapper.find('[data-testid="add-field"]').trigger('click')
      await clickApply(wrapper)
      expect(wrapper.text()).toContain('Every field needs a name.')
      expect(wrapper.emitted('update')).toBeFalsy()
    })
  })

  describe('edits are never lost', () => {
    afterEach(() => {
      vi.useRealTimers()
    })
    const agentNode = (): NodeInstance => ({ id: 'ag', node_type: 'ai.agent', position: [0, 0], parameters: { model: 'm', user_message: 'hi' }, disabled: false })

    it('applies an edit by itself shortly after typing stops, without Apply', async () => {
      vi.useFakeTimers()
      const wrapper = mount(NodeConfigPanel, { props: { node: agentNode() } })
      await wrapper.find('textarea[aria-label="System prompt"]').setValue('You are brief.')
      expect(wrapper.emitted('update')).toBeFalsy()
      await vi.advanceTimersByTimeAsync(600)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.system_prompt).toBe('You are brief.')
    })

    it('flush() applies a pending edit at once (Save, Execute, switching nodes call it)', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agentNode() } })
      await wrapper.find('textarea[aria-label="System prompt"]').setValue('Be formal.')
      expect((wrapper.vm as unknown as { flush: () => boolean }).flush()).toBe(true)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.system_prompt).toBe('Be formal.')
    })

    it('flush() with nothing changed emits nothing', () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agentNode() } })
      expect((wrapper.vm as unknown as { flush: () => boolean }).flush()).toBe(true)
      expect(wrapper.emitted('update')).toBeFalsy()
    })

    it('flush() refuses an invalid edit and says why, instead of dropping it', async () => {
      vi.useFakeTimers()
      const wrapper = mount(NodeConfigPanel, { props: { node: agentNode() } })
      await wrapper.find('input[aria-label="Model"]').setValue('')
      await vi.advanceTimersByTimeAsync(600)
      expect(wrapper.emitted('update')).toBeFalsy() // not applied while invalid, no error mid-typing
      expect(wrapper.text()).not.toContain('Model is required.')
      expect((wrapper.vm as unknown as { flush: () => boolean }).flush()).toBe(false)
      await wrapper.vm.$nextTick()
      expect(wrapper.text()).toContain('Model is required.')
    })
  })

  it('Delete asks to remove the node, without applying edits', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="delete-node"]').trigger('click')
    expect(wrapper.emitted('delete')).toEqual([['n1']])
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('choosing "No credential" removes the stored credential on Apply', async () => {
    const withCred: NodeInstance = {
      ...node,
      parameters: { foo: 'bar', auth: { credential_id: 'cred-1' } },
    }
    const wrapper = mount(NodeConfigPanel, { props: { node: withCred } })
    await wrapper.find('select').setValue('')
    await clickApply(wrapper)
    const events = wrapper.emitted('update')
    expect((events![0][0] as NodeInstance).parameters).toEqual({ foo: 'bar' })
  })

  function emittedSettings(wrapper: ReturnType<typeof mount>): NodeSettings | undefined {
    const events = wrapper.emitted('update')
    return events ? (events[0][0] as NodeInstance).settings : undefined
  }

  it('loads a node without settings with retry off and default retry values', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    expect((wrapper.find('[data-testid="retry-enabled"]').element as HTMLInputElement).checked).toBe(false)
    expect(wrapper.find('[data-testid="max-tries"]').exists()).toBe(false)
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    expect((wrapper.find('[data-testid="max-tries"]').element as HTMLInputElement).value).toBe('3')
    expect((wrapper.find('[data-testid="wait-ms"]').element as HTMLInputElement).value).toBe('1000')
  })

  it('emits default settings when nothing is changed', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)).toEqual({ retry: null, timeout_ms: null, continue_on_fail: false })
  })

  it('emits the configured settings on Apply', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="continue-on-fail"]').setValue(true)
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="max-tries"]').setValue('5')
    await wrapper.find('[data-testid="wait-ms"]').setValue('250')
    await wrapper.find('[data-testid="timeout-ms"]').setValue('3000')
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)).toEqual({ retry: { max_tries: 5, wait_ms: 250 }, timeout_ms: 3000, continue_on_fail: true })
  })

  it('emits timeout_ms null when the timeout field is cleared', async () => {
    const withTimeout: NodeInstance = { ...node, settings: { retry: null, timeout_ms: 500, continue_on_fail: false } }
    const wrapper = mount(NodeConfigPanel, { props: { node: withTimeout } })
    await wrapper.find('[data-testid="timeout-ms"]').setValue('')
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)?.timeout_ms).toBeNull()
  })

  it('preserves a stored wait of 0 ms', async () => {
    const zeroWait: NodeInstance = { ...node, settings: { retry: { max_tries: 4, wait_ms: 0 }, timeout_ms: null, continue_on_fail: false } }
    const wrapper = mount(NodeConfigPanel, { props: { node: zeroWait } })
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)?.retry).toEqual({ max_tries: 4, wait_ms: 0 })
  })

  it('rejects a cleared wait field instead of silently saving 0 ms', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="wait-ms"]').setValue('')
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Wait between tries must be between 0 and 60000 ms.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('shows an error and emits nothing for out-of-range max tries', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="max-tries"]').setValue('11')
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Max tries must be between 2 and 10.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  function seedCredential(id: string, credentialType: string) {
    const store = useCredentialsStore()
    store.credentials = [{ id, name: 'cred', credential_type: credentialType, owner_id: 'u', created_at: '', updated_at: '', used_by: 0 }]
    store.loaded = true
  }

  describe('Credential section', () => {
    async function withTypes(types: Record<string, string[]>) {
      const { useNodeTypesStore } = await import('../stores/nodeTypes')
      const store = useNodeTypesStore()
      store.types = Object.entries(types).map(([type_name, credential_types]) => ({
        type_name, display_name: type_name, icon: '', category: 'action', description: '', credential_types, output_ports: ['main'],
      }))
      store.loaded = true
    }
    const at = (node_type: string, parameters: Record<string, unknown> = {}): NodeInstance => ({ id: 'n', node_type, position: [0, 0], parameters, disabled: false })
    const shown = (w: ReturnType<typeof mount>) => w.find('[data-testid="credential-section"]').exists()

    it('is hidden for nodes that take no credential (Code, Loop, Set, ...)', async () => {
      await withTypes({ 'core.code': [], 'core.loop': [], 'core.set': [] })
      for (const type of ['core.code', 'core.loop', 'core.set']) {
        expect(shown(mount(NodeConfigPanel, { props: { node: at(type) } }))).toBe(false)
      }
    })

    it('is shown for nodes that take one', async () => {
      await withTypes({ 'ai.agent': ['openaiApi'], 'core.httpRequest': ['bearerToken'] })
      expect(shown(mount(NodeConfigPanel, { props: { node: at('ai.agent') } }))).toBe(true)
      expect(shown(mount(NodeConfigPanel, { props: { node: at('core.httpRequest') } }))).toBe(true)
    })

    it('stays for a node that still holds a credential, so it can be removed', async () => {
      await withTypes({ 'core.code': [] })
      expect(shown(mount(NodeConfigPanel, { props: { node: at('core.code', { auth: { credential_id: 'old' } }) } }))).toBe(true)
    })
  })

  function agent(parameters: Record<string, unknown>): NodeInstance {
    return { id: 'a1', node_type: 'ai.agent', position: [0, 0], parameters: { model: 'm', user_message: 'hi', ...parameters }, disabled: false }
  }

  it('fills in the AI Agent provider from the selected credential type on Apply', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' } }) } })
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.provider).toBe('openai')
  })

  it('keeps an explicitly set AI Agent provider', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' }, provider: 'anthropic' }) } })
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.provider).toBe('anthropic')
  })

  it('does not add a provider to other node types', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: { ...node, parameters: { auth: { credential_id: 'c1' } } } } })
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.provider).toBeUndefined()
  })

  it('loads existing agent parameters into the form', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' }, provider: 'openai', model: 'qwen', system_prompt: 'be brief', user_message: '{{ $json.text }}', max_iterations: 5 }) } })
    expect((wrapper.find('select[aria-label="Provider"]').element as HTMLSelectElement).value).toBe('openai')
    expect((wrapper.find('input[aria-label="Model"]').element as HTMLInputElement).value).toBe('qwen')
    expect((wrapper.find('textarea[aria-label="System prompt"]').element as HTMLTextAreaElement).value).toBe('be brief')
    expect((wrapper.find('textarea[aria-label="User message"]').element as HTMLTextAreaElement).value).toBe('{{ $json.text }}')
    expect((wrapper.find('input[aria-label="Max iterations"]').element as HTMLInputElement).value).toBe('5')
  })

  it('writes the form fields into the agent parameters on Apply', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' } }) } })
    await wrapper.find('input[aria-label="Model"]').setValue('llama3')
    await wrapper.find('textarea[aria-label="User message"]').setValue('Hello')
    await wrapper.find('textarea[aria-label="System prompt"]').setValue('sys')
    await clickApply(wrapper)
    const params = (wrapper.emitted('update')![0][0] as NodeInstance).parameters
    expect(params).toMatchObject({ provider: 'openai', model: 'llama3', user_message: 'Hello', system_prompt: 'sys', max_iterations: 10, tool_ids: [] })
  })

  it('keeps a long, multi-line prompt as written', async () => {
    const prompt = 'You are a helpful assistant.\n\nRules:\n1. Be brief.\n2. Answer in the user\'s language.\n\nQuestion: {{ $json.message.text }}'
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm' }) } })
    await wrapper.find('textarea[aria-label="User message"]').setValue(prompt)
    await wrapper.find('textarea[aria-label="System prompt"]').setValue('Line one\nLine two')
    await clickApply(wrapper)
    const params = (wrapper.emitted('update')![0][0] as NodeInstance).parameters
    expect(params.user_message).toBe(prompt)
    expect(params.system_prompt).toBe('Line one\nLine two')
  })

  it('edits a prompt in a large editor and shows its length', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'short' }) } })
    expect(wrapper.find('[data-testid="prompt-editor"]').exists()).toBe(false)
    await wrapper.find('[data-testid="expand-User message"]').trigger('click')
    const big = wrapper.find('[data-testid="prompt-editor"] textarea')
    expect((big.element as HTMLTextAreaElement).value).toBe('short')
    await big.setValue('a much longer prompt\nwith two lines')
    expect(wrapper.find('[data-testid="prompt-editor"]').text()).toContain('35 characters')
    await wrapper.find('[data-testid="prompt-editor-done"]').trigger('click')
    expect(wrapper.find('[data-testid="prompt-editor"]').exists()).toBe(false)
    expect((wrapper.find('textarea[aria-label="User message"]').element as HTMLTextAreaElement).value).toBe('a much longer prompt\nwith two lines')
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.user_message).toBe('a much longer prompt\nwith two lines')
  })

  it('scrolls to and highlights the section a port asked for', async () => {
    const scrolled: string[] = []
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.getAttribute('data-section') ?? '')
    }
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'u' }), focus: { section: 'memory', at: 1 } }, attachTo: document.body })
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(scrolled).toEqual(['memory'])
    expect(wrapper.find('[data-section="memory"]').classes()).toContain('ring-2')
    wrapper.unmount()
  })

  describe('Code node', () => {
    const codeNode = (parameters: Record<string, unknown>): NodeInstance => ({ id: 'c1', node_type: 'core.code', position: [0, 0], parameters, disabled: false })
    const lang = (w: ReturnType<typeof mount>) => w.find('select[aria-label="Language"]')
    const code = (w: ReturnType<typeof mount>) => w.find('textarea[aria-label="Code"]')
    const applied = (w: ReturnType<typeof mount>) => (w.emitted('update')![0][0] as NodeInstance).parameters

    it('starts a new Code node in JavaScript with a working example', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({}) } })
      expect((lang(wrapper).element as HTMLSelectElement).value).toBe('javaScript')
      expect((code(wrapper).element as HTMLTextAreaElement).value).toContain('return items.map(')
      await clickApply(wrapper)
      expect(applied(wrapper)).toMatchObject({ language: 'javaScript' })
      expect(applied(wrapper).script).toContain('return items.map(')
    })

    it('saves the example of a new node by itself, so it runs as shown', async () => {
      vi.useFakeTimers()
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({}) } })
      await vi.advanceTimersByTimeAsync(600)
      expect(applied(wrapper).script).toContain('return items.map(')
      vi.useRealTimers()
    })

    it('switches an untouched example to Python', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({}) } })
      await lang(wrapper).setValue('python')
      expect((code(wrapper).element as HTMLTextAreaElement).value).toContain('for item in items')
      await clickApply(wrapper)
      expect(applied(wrapper)).toMatchObject({ language: 'python' })
    })

    it('never replaces code the user wrote when the language changes', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({ script: 'return items' }) } })
      expect((lang(wrapper).element as HTMLSelectElement).value).toBe('javaScript') // older nodes have no language
      await lang(wrapper).setValue('python')
      expect((code(wrapper).element as HTMLTextAreaElement).value).toBe('return items')
      expect(wrapper.text()).toContain('written in JavaScript')
    })

    it('loads and saves Python code', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({ language: 'python', script: 'return items' }) } })
      expect((lang(wrapper).element as HTMLSelectElement).value).toBe('python')
      await code(wrapper).setValue('return [{"ok": True}]')
      await clickApply(wrapper)
      expect(applied(wrapper)).toEqual({ language: 'python', script: 'return [{"ok": True}]' })
    })

    it('inserts a field from the previous nodes at the cursor, in the code\'s language', async () => {
      const sources = [
        { nodeId: 'agent', direct: true, fields: [{ path: '.message.chat.id', preview: '42', segments: ['message', 'chat', 'id'], expression: '' }] },
        { nodeId: 'tg', direct: false, fields: [{ path: '.update_id', preview: '9', segments: ['update_id'], expression: '' }] },
      ]
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({ script: 'const id = ;' }), inputSources: sources, nodeLabels: { agent: 'AI Agent', tg: 'Telegram Trigger' } }, attachTo: document.body })
      const el = code(wrapper).element as HTMLTextAreaElement
      el.setSelectionRange(11, 11)
      await wrapper.find('[data-testid="insert-field"]').trigger('click')
      await wrapper.find('[data-testid="code-field-picker"]').findAll('button').find((b) => b.text().startsWith('message.chat.id'))!.trigger('click')
      expect(el.value).toBe('const id = $json.message.chat.id;')

      await lang(wrapper).setValue('python')
      el.setSelectionRange(0, 0)
      await wrapper.find('[data-testid="insert-field"]').trigger('click')
      await wrapper.find('[data-testid="code-field-picker"]').findAll('button').find((b) => b.text().startsWith('update_id'))!.trigger('click')
      expect(el.value.startsWith('_node["tg"]["json"]["update_id"]')).toBe(true)
      wrapper.unmount()
    })

    it('indents with Tab instead of leaving the box', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: codeNode({ script: 'ab' }) } })
      const el = code(wrapper).element as HTMLTextAreaElement
      el.setSelectionRange(1, 1)
      await code(wrapper).trigger('keydown', { key: 'Tab' })
      expect(el.value).toBe('a  b')
    })
  })

  describe('Loop Over Items', () => {
    const loopNode = (parameters: Record<string, unknown>): NodeInstance => ({ id: 'l1', node_type: 'core.loop', position: [0, 0], parameters, disabled: false })
    const size = (w: ReturnType<typeof mount>) => w.find('input[aria-label="Items per batch"]')

    it('starts at one item per batch and saves the size', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: loopNode({}) } })
      expect((size(wrapper).element as HTMLInputElement).value).toBe('1')
      await size(wrapper).setValue('10')
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.batch_size).toBe(10)
    })

    it('refuses a size below 1', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: loopNode({ batch_size: 3 }) } })
      expect((size(wrapper).element as HTMLInputElement).value).toBe('3')
      await size(wrapper).setValue('0')
      await clickApply(wrapper)
      expect(wrapper.text()).toContain('Items per batch must be a whole number of at least 1.')
    })
  })

  describe('agent memory', () => {
    it('is off by default and adds nothing to an agent that never had it', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'u' }) } })
      expect((wrapper.find('input[aria-label="Remember the conversation"]').element as HTMLInputElement).checked).toBe(false)
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.memory).toBeUndefined()
    })

    it('saves the window and session key when turned on', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'u' }) } })
      await wrapper.find('input[aria-label="Remember the conversation"]').setValue(true)
      await wrapper.find('input[aria-label="Exchanges to remember"]').setValue('8')
      await wrapper.find('input[aria-label="Session key"]').setValue('{{ $json.message.from.id }}')
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.memory).toEqual({ enabled: true, window: 8, session_key: '{{ $json.message.from.id }}' })
    })

    it('loads saved memory settings, and keeps them (off) when turned off', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'u', memory: { enabled: true, window: 3, session_key: '' } }) } })
      expect((wrapper.find('input[aria-label="Exchanges to remember"]').element as HTMLInputElement).value).toBe('3')
      await wrapper.find('input[aria-label="Remember the conversation"]').setValue(false)
      await clickApply(wrapper)
      expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.memory).toEqual({ enabled: false, window: 3, session_key: '' })
    })

    it('refuses a window outside 1..50', async () => {
      const wrapper = mount(NodeConfigPanel, { props: { node: agent({ model: 'm', user_message: 'u' }) } })
      await wrapper.find('input[aria-label="Remember the conversation"]').setValue(true)
      await wrapper.find('input[aria-label="Exchanges to remember"]').setValue('0')
      await clickApply(wrapper)
      expect(wrapper.text()).toContain('Exchanges to remember must be between 1 and 50.')
    })
  })

  it('requires model and user message for an agent', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node: { id: 'a1', node_type: 'ai.agent', position: [0, 0], parameters: {}, disabled: false } } })
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Model is required.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('stores checked library tools as tool_ids', async () => {
    const { useToolsStore } = await import('../stores/tools')
    const tools = useToolsStore()
    tools.tools = [{ id: 't1', name: 'search', description: 'web search', node_type: 'core.httpRequest', argument_schema: { type: 'object' }, parameters: {}, created_at: '', updated_at: '' }]
    tools.loaded = true
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({}) } })
    await wrapper.find('input[aria-label="Use tool search"]').setValue(true)
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.tool_ids).toEqual(['t1'])
  })

  it('opens Manage tools in a new tab so unsaved canvas edits are not lost', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({}) }, global: { stubs: { RouterLink: { template: '<a v-bind="$attrs"><slot /></a>' } } } })
    const link = wrapper.find('[data-testid="manage-tools"]')
    expect(link.exists()).toBe(true)
    expect(link.attributes('target')).toBe('_blank')
  })

  it('refreshes the tool list each time an agent panel opens', async () => {
    const { useToolsStore } = await import('../stores/tools')
    const tools = useToolsStore()
    tools.loaded = true
    const spy = vi.spyOn(tools, 'fetchAll').mockResolvedValue()
    mount(NodeConfigPanel, { props: { node: agent({}) } })
    expect(spy).toHaveBeenCalled()
  })
})
