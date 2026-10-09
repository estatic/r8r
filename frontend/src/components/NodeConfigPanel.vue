<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { AgentFields, NodeInstance, NodeSettings } from '../types/domain'
import CredentialPicker from './CredentialPicker.vue'
import AgentSettings from './AgentSettings.vue'
import TelegramTriggerSettings from './TelegramTriggerSettings.vue'
import SetFieldsEditor from './SetFieldsEditor.vue'
import SetOptionsEditor from './SetOptionsEditor.vue'
import ConditionsEditor from './ConditionsEditor.vue'
import FirecrawlSettings from './FirecrawlSettings.vue'
import HttpRequestSettings from './HttpRequestSettings.vue'
import SwitchSettings from './SwitchSettings.vue'
import MergeSettings from './MergeSettings.vue'
import ScheduleSettings from './ScheduleSettings.vue'
import WebhookSettings from './WebhookSettings.vue'
import { WEBHOOK_KEYS, buildWebhook, loadWebhook, type WebhookForm } from '../canvas/webhook'
import WaitSettings from './WaitSettings.vue'
import { SCHEDULE_KEYS, WAIT_KEYS, buildSchedule, buildWait, loadSchedule, loadWait, type ScheduleForm, type WaitForm } from '../canvas/schedule'
import { MERGE_KEYS, buildMerge, loadMerge, type MergeForm } from '../canvas/merge'
import { SWITCH_KEYS, buildSwitch, loadSwitch, type SwitchForm } from '../canvas/switchRules'
import { HTTP_KEYS, buildHttp, loadHttp, type HttpForm } from '../canvas/httpRequest'
import { authForCredentialType } from '../tools/schema'
import { FIRECRAWL_KEYS, buildFirecrawl, loadFirecrawl, type FirecrawlForm } from '../canvas/firecrawl'
import { buildConditions, loadConditions, type ConditionsForm } from '../canvas/conditions'
import CodeSettings from './CodeSettings.vue'
import TelegramSendSettings from './TelegramSendSettings.vue'
import { buildMessage, loadMessage, type MessageForm } from '../canvas/telegramMessage'
import { codeExample, type CodeLanguage, type CodeMode } from '../canvas/codeExamples'
import { buildFields, loadRows, type SetFieldRow, SET_OPTION_KEYS, buildSetOptions, loadSetOptions, type SetOptions } from '../canvas/setFields'
import type { UpstreamSource } from '../canvas/inputData'
import { useCredentialsStore } from '../stores/credentials'
import { useNodeTypesStore } from '../stores/nodeTypes'

// ai.agent reads `provider` from its parameters, but the credential type
// already says which API it is for; fill it in when the user hasn't.
const PROVIDER_BY_CREDENTIAL_TYPE: Record<string, string> = {
  openaiApi: 'openai',
  anthropicApi: 'anthropic',
}

const props = withDefaults(
  defineProps<{
    node: NodeInstance | null
    /** The data reaching the node, for the Set form's picker. */
    inputSources?: UpstreamSource[]
    nodeLabels?: Record<string, string>
    /** The workflow's id, for the Webhook's URL. */
    workflowId?: string
    /** A section to bring into view (an agent port was clicked); `at` makes repeats count. */
    focus?: { section: string; at: number } | null
  }>(),
  { inputSources: () => [], nodeLabels: () => ({}) },
)
const emit = defineEmits<{ update: [node: NodeInstance]; close: []; delete: [nodeId: string] }>()

const paramsText = ref('')
const error = ref('')
const disabled = ref(false)
const credentialId = ref<string | null>(null)
const credentialsStore = useCredentialsStore()
const continueOnFail = ref(false)
const retryEnabled = ref(false)
const maxTries = ref<number | string>(3)
const waitMs = ref<number | string>(1000)
// '' = no timeout (an empty <input type="number">).
const timeoutMs = ref<number | string>('')

const isAgent = computed(() => props.node?.node_type === 'ai.agent')
const emptyAgentFields = (): AgentFields => ({
  provider: '',
  model: '',
  system_prompt: '',
  user_message: '',
  max_iterations: 10,
  tool_ids: [],
  memory_enabled: false,
  memory_window: 5,
  memory_session_key: '',
  memory_stored: false,
})
const agentFields = ref<AgentFields>(emptyAgentFields())

const isCode = computed(() => props.node?.node_type === 'core.code')
const codeFields = ref<{ mode: CodeMode; language: CodeLanguage; script: string; writtenIn: CodeLanguage }>({
  mode: 'runOnceForAllItems',
  language: 'javaScript',
  script: '',
  writtenIn: 'javaScript',
})
function loadCode(parameters: Record<string, unknown>) {
  // Nodes saved before the language and mode choices are JavaScript, run once for all items.
  const language: CodeLanguage = parameters.language === 'python' ? 'python' : 'javaScript'
  const mode: CodeMode = parameters.mode === 'runOnceForEachItem' ? 'runOnceForEachItem' : 'runOnceForAllItems'
  const script = typeof parameters.script === 'string' ? parameters.script : codeExample(language, mode)
  return { mode, language, script, writtenIn: language }
}

// Only nodes that take a credential show the picker (a node that still holds
// one keeps it, so it can be removed). Until the types load, it shows.
const nodeTypesStore = useNodeTypesStore()
const takesCredential = computed(() => {
  if (credentialId.value) return true
  const meta = nodeTypesStore.types.find((t) => t.type_name === props.node?.node_type)
  return !nodeTypesStore.loaded || !meta || meta.credential_types.length > 0
})

const isTelegramSend = computed(() => props.node?.node_type === 'telegram.sendMessage')
const telegramMessage = ref<MessageForm>(loadMessage({}))
// The keys the Send Message form owns (it rewrites them all on Apply).
const TELEGRAM_MESSAGE_KEYS = ['chat_id', 'text', 'parse_mode', 'reply_markup', 'disable_notification', 'protect_content', 'disable_web_page_preview', 'reply_to_message_id', 'message_thread_id']

const isLoop = computed(() => props.node?.node_type === 'core.loop')
const batchSize = ref<number | string>(1)

const isSet = computed(() => props.node?.node_type === 'core.set')
const setRows = ref<SetFieldRow[]>([])
const setOptions = ref<SetOptions>(loadSetOptions({}))

const hasConditions = computed(() => props.node?.node_type === 'core.if' || props.node?.node_type === 'core.filter')
const conditions = ref<ConditionsForm>(loadConditions({}))
const isWebhook = computed(() => props.node?.node_type === 'core.webhook')
const webhookForm = ref<WebhookForm>(loadWebhook({}))
const isSchedule = computed(() => props.node?.node_type === 'core.schedule')
const scheduleForm = ref<ScheduleForm>(loadSchedule({}))
const isWait = computed(() => props.node?.node_type === 'core.wait')
const waitForm = ref<WaitForm>(loadWait({}))
const isMerge = computed(() => props.node?.node_type === 'core.merge')
const mergeForm = ref<MergeForm>(loadMerge({}))
const isSwitch = computed(() => props.node?.node_type === 'core.switch')
const switchForm = ref<SwitchForm>(loadSwitch({}))
const isHttp = computed(() => props.node?.node_type === 'core.httpRequest')
const http = ref<HttpForm>(loadHttp({}))
const isFirecrawl = computed(() => props.node?.node_type === 'firecrawl.search')
const firecrawl = ref<FirecrawlForm>(loadFirecrawl({}))
// Nodes with their own form keep the raw JSON under "Advanced".
const hasForm = computed(() => isAgent.value || isSet.value || isCode.value || isLoop.value || isTelegramSend.value || hasConditions.value || isFirecrawl.value || isHttp.value || isSwitch.value || isMerge.value || isSchedule.value || isWait.value || isWebhook.value)

const isTelegramTrigger = computed(() => props.node?.node_type === 'telegram.trigger')
// Nothing chosen yet means every update, as the trigger treats it.
const telegramUpdates = ref<string[]>(['*'])
// n8n's "Restrict to Chat IDs / User IDs", comma-separated.
const telegramRestrict = ref({ chats: '', users: '' })
const idList = (v: unknown) => (Array.isArray(v) ? v.join(', ') : typeof v === 'string' || typeof v === 'number' ? String(v) : '')
function loadTelegramUpdates(parameters: Record<string, unknown>): string[] {
  const u = parameters.updates
  const chosen = Array.isArray(u) ? u.filter((v): v is string => typeof v === 'string') : []
  return chosen.length > 0 ? chosen : ['*']
}
const inlineToolCount = computed(() => {
  const tools = props.node?.parameters?.tools
  return Array.isArray(tools) ? tools.length : 0
})

function loadAgentFields(parameters: Record<string, unknown>): AgentFields {
  const str = (k: string) => (typeof parameters[k] === 'string' ? (parameters[k] as string) : '')
  const ids = parameters.tool_ids
  return {
    provider: str('provider'),
    model: str('model'),
    system_prompt: str('system_prompt'),
    user_message: str('user_message'),
    max_iterations: typeof parameters.max_iterations === 'number' ? parameters.max_iterations : 10,
    tool_ids: Array.isArray(ids) ? ids.filter((v): v is string => typeof v === 'string') : [],
    ...loadMemory(parameters.memory),
  }
}

function loadMemory(memory: unknown): Pick<AgentFields, 'memory_enabled' | 'memory_window' | 'memory_session_key' | 'memory_stored'> {
  const m = memory && typeof memory === 'object' ? (memory as Record<string, unknown>) : null
  return {
    memory_enabled: m?.enabled === true,
    memory_window: typeof m?.window === 'number' ? m.window : 5,
    memory_session_key: typeof m?.session_key === 'string' ? m.session_key : '',
    memory_stored: m !== null,
  }
}

// Everything the form holds, to tell whether it differs from the node.
const formSnapshot = computed(() =>
  JSON.stringify([
    paramsText.value,
    disabled.value,
    continueOnFail.value,
    retryEnabled.value,
    maxTries.value,
    waitMs.value,
    timeoutMs.value,
    credentialId.value,
    agentFields.value,
    codeFields.value.mode,
    codeFields.value.language,
    codeFields.value.script,
    batchSize.value,
    isTelegramSend.value ? telegramMessage.value : null,
    telegramUpdates.value,
    isTelegramTrigger.value ? telegramRestrict.value : null,
    setRows.value,
    isSet.value ? setOptions.value : null,
    hasConditions.value ? conditions.value : null,
    isFirecrawl.value ? firecrawl.value : null,
    isHttp.value ? http.value : null,
    isSwitch.value ? switchForm.value : null,
    isMerge.value ? mergeForm.value : null,
    isSchedule.value ? scheduleForm.value : null,
    isWebhook.value ? webhookForm.value : null,
    isWait.value ? waitForm.value : null,
  ]),
)
const loaded = ref('')
const pending = computed(() => props.node !== null && formSnapshot.value !== loaded.value)

watch(
  () => props.node,
  (node) => {
    if (node) {
      paramsText.value = JSON.stringify(node.parameters, null, 2)
      disabled.value = node.disabled
      continueOnFail.value = node.settings?.continue_on_fail ?? false
      retryEnabled.value = !!node.settings?.retry
      maxTries.value = node.settings?.retry?.max_tries ?? 3
      waitMs.value = node.settings?.retry?.wait_ms ?? 1000
      timeoutMs.value = node.settings?.timeout_ms ?? ''
      error.value = ''
      const auth = node.parameters?.auth as { credential_id?: string } | undefined
      credentialId.value = auth?.credential_id ?? null
      agentFields.value = node.node_type === 'ai.agent' ? loadAgentFields(node.parameters ?? {}) : emptyAgentFields()
      telegramUpdates.value = loadTelegramUpdates(node.parameters ?? {})
      telegramRestrict.value = { chats: idList(node.parameters?.restrict_chat_ids), users: idList(node.parameters?.restrict_user_ids) }
      setRows.value = node.node_type === 'core.set' ? loadRows(node.parameters?.fields) : []
      setOptions.value = loadSetOptions(node.node_type === 'core.set' ? (node.parameters ?? {}) : {})
      webhookForm.value = loadWebhook(node.node_type === 'core.webhook' ? (node.parameters ?? {}) : {})
      scheduleForm.value = loadSchedule(node.node_type === 'core.schedule' ? (node.parameters ?? {}) : {})
      waitForm.value = loadWait(node.node_type === 'core.wait' ? (node.parameters ?? {}) : {})
      mergeForm.value = loadMerge(node.node_type === 'core.merge' ? (node.parameters ?? {}) : {})
      switchForm.value = loadSwitch(node.node_type === 'core.switch' ? (node.parameters ?? {}) : {})
      http.value = loadHttp(node.node_type === 'core.httpRequest' ? (node.parameters ?? {}) : {})
      firecrawl.value = loadFirecrawl(node.node_type === 'firecrawl.search' ? (node.parameters ?? {}) : {})
      conditions.value = loadConditions(node.node_type === 'core.if' || node.node_type === 'core.filter' ? (node.parameters ?? {}) : {})
      if (node.node_type === 'core.code') codeFields.value = loadCode(node.parameters ?? {})
      batchSize.value = typeof node.parameters?.batch_size === 'number' ? node.parameters.batch_size : 1
      telegramMessage.value = loadMessage(node.node_type === 'telegram.sendMessage' ? (node.parameters ?? {}) : {})
      loaded.value = formSnapshot.value
      // A new Code node shows its example: save it, so the node can run as shown.
      if (node.node_type === 'core.code' && typeof node.parameters?.script !== 'string') {
        loaded.value = ''
        nextTick(() => scheduleAutoApply())
      }
    }
  },
  { immediate: true },
)

// Mirrors the backend's validate_nodes ranges; the backend 400 stays the authority.
function buildSettings(): NodeSettings | string {
  // A cleared field is NaN (rejected below), not Number('') === 0.
  const num = (v: number | string) => (v === '' ? NaN : Number(v))
  const retry = retryEnabled.value ? { max_tries: num(maxTries.value), wait_ms: num(waitMs.value) } : null
  if (retry && !(Number.isInteger(retry.max_tries) && retry.max_tries >= 2 && retry.max_tries <= 10)) {
    return 'Max tries must be between 2 and 10.'
  }
  if (retry && !(Number.isInteger(retry.wait_ms) && retry.wait_ms >= 0 && retry.wait_ms <= 60000)) {
    return 'Wait between tries must be between 0 and 60000 ms.'
  }
  const timeout = timeoutMs.value === '' ? null : Number(timeoutMs.value)
  if (timeout !== null && !(Number.isInteger(timeout) && timeout >= 1 && timeout <= 3600000)) {
    return 'Timeout must be between 1 and 3600000 ms.'
  }
  return { retry, timeout_ms: timeout, continue_on_fail: continueOnFail.value }
}

/** The node as the form describes it, or why it can't be. */
function build(): NodeInstance | string {
  if (!props.node) return 'No node.'
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(paramsText.value)
  } catch {
    return 'Parameters must be valid JSON.'
  }
  if (credentialId.value && isHttp.value) {
    // HTTP Request sends a credential as its type says (bearer, API key header, basic).
    const type = credentialsStore.credentials.find((c) => c.id === credentialId.value)?.credential_type
    parsed.auth = { ...((parsed.auth as object) ?? {}), ...authForCredentialType('core.httpRequest', credentialId.value, type) }
  } else if (credentialId.value) {
    parsed.auth = { ...((parsed.auth as object) ?? {}), credential_id: credentialId.value }
  } else if (parsed.auth && typeof parsed.auth === 'object') {
    // "No credential": drop the stored one, or the JSON would bring it back
    // (and HTTP Request's auth type, which would then lack its credential).
    const { credential_id: _dropped, ...rest } = parsed.auth as Record<string, unknown>
    if (isHttp.value) delete rest.type
    if (Object.keys(rest).length > 0) parsed.auth = rest
    else delete parsed.auth
  }
  if (isTelegramSend.value) {
    const built = buildMessage(telegramMessage.value)
    if ('error' in built) return built.error
    for (const key of TELEGRAM_MESSAGE_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isLoop.value) {
    const n = Number(batchSize.value)
    if (!Number.isInteger(n) || n < 1) return 'Items per batch must be a whole number of at least 1.'
    parsed.batch_size = n
  }
  if (isCode.value) {
    parsed.language = codeFields.value.language
    parsed.script = codeFields.value.script
    // "Run once for all items" is the default, so it isn't stored.
    if (codeFields.value.mode === 'runOnceForEachItem') parsed.mode = 'runOnceForEachItem'
    else delete parsed.mode
  }
  if (isWebhook.value) {
    const built = buildWebhook(webhookForm.value)
    if ('error' in built) return built.error
    for (const key of WEBHOOK_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isSchedule.value) {
    const built = buildSchedule(scheduleForm.value)
    if ('error' in built) return built.error
    for (const key of SCHEDULE_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isWait.value) {
    const built = buildWait(waitForm.value)
    if ('error' in built) return built.error
    for (const key of WAIT_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isMerge.value) {
    const built = buildMerge(mergeForm.value)
    if ('error' in built) return built.error
    for (const key of MERGE_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isSwitch.value) {
    const built = buildSwitch(switchForm.value)
    if ('error' in built) return built.error
    for (const key of SWITCH_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isHttp.value) {
    const built = buildHttp(http.value)
    if ('error' in built) return built.error
    for (const key of HTTP_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (isFirecrawl.value) {
    const built = buildFirecrawl(firecrawl.value)
    if ('error' in built) return built.error
    for (const key of FIRECRAWL_KEYS) delete parsed[key]
    Object.assign(parsed, built.fields)
  }
  if (hasConditions.value) {
    const built = buildConditions(conditions.value)
    if ('error' in built) return built.error
    // The rules replace the older single `condition`.
    delete parsed.condition
    parsed.conditions = built.conditions
  }
  if (isSet.value) {
    const options = buildSetOptions(setOptions.value)
    if ('error' in options) return options.error
    for (const key of SET_OPTION_KEYS) delete parsed[key]
    Object.assign(parsed, options.fields)
  }
  if (isSet.value && setOptions.value.mode === 'manual') {
    const built = buildFields(setRows.value)
    if ('error' in built) return built.error
    // Don't invent an empty `fields` the node never had.
    if (setRows.value.length > 0 || 'fields' in parsed) parsed.fields = built.fields
  }
  if (isTelegramTrigger.value) {
    if (telegramUpdates.value.length === 0) return 'Choose at least one update type, or All updates.'
    parsed.updates = [...telegramUpdates.value]
    for (const [key, text] of [['restrict_chat_ids', telegramRestrict.value.chats], ['restrict_user_ids', telegramRestrict.value.users]] as const) {
      const ids = text.split(',').map((s) => s.trim()).filter(Boolean)
      if (ids.some((id) => !/^-?\d+$/.test(id))) return 'Chat and user IDs are numbers, separated by commas.'
      if (ids.length > 0) parsed[key] = ids
      else delete parsed[key]
    }
  }
  if (isAgent.value) {
    // The form is the source of truth for the fields it shows; everything
    // else in the JSON (inline tools, api_base_url, ...) is kept as typed.
    const f = agentFields.value
    const iterations = Number(f.max_iterations)
    if (!f.model.trim()) return 'Model is required.'
    if (!f.user_message.trim()) return 'User message is required.'
    if (!Number.isInteger(iterations) || iterations < 1 || iterations > 50) return 'Max iterations must be between 1 and 50.'
    parsed.model = f.model.trim()
    parsed.user_message = f.user_message
    parsed.system_prompt = f.system_prompt
    parsed.max_iterations = iterations
    parsed.tool_ids = f.tool_ids
    if (f.memory_enabled || f.memory_stored) {
      const window = Number(f.memory_window)
      if (f.memory_enabled && (!Number.isInteger(window) || window < 1 || window > 50)) return 'Exchanges to remember must be between 1 and 50.'
      parsed.memory = { enabled: f.memory_enabled, window: Number.isInteger(window) ? window : 5, session_key: f.memory_session_key.trim() }
    }
    if (f.provider) parsed.provider = f.provider
    else delete parsed.provider
  }
  if (props.node.node_type === 'ai.agent' && typeof parsed.provider !== 'string' && credentialId.value) {
    const type = credentialsStore.credentials.find((c) => c.id === credentialId.value)?.credential_type
    const provider = type ? PROVIDER_BY_CREDENTIAL_TYPE[type] : undefined
    if (provider) parsed.provider = provider
  }
  const settings = buildSettings()
  if (typeof settings === 'string') return settings
  return { ...props.node, parameters: parsed, disabled: disabled.value, settings }
}

/** Applies the form to the node now; false (with the reason shown) if it's invalid. */
function apply(): boolean {
  const node = build()
  if (typeof node === 'string') {
    error.value = node
    return false
  }
  error.value = ''
  loaded.value = formSnapshot.value
  emit('update', node)
  return true
}

/** Applies pending edits, if any. Callers stop (Save, Execute, ...) on false. */
function flush(): boolean {
  return pending.value ? apply() : true
}
defineExpose({ flush })

const panelBody = ref<HTMLElement | null>(null)
watch(
  () => props.focus,
  async (focus) => {
    if (!focus) return
    await nextTick()
    const el = panelBody.value?.querySelector<HTMLElement>(`[data-section="${focus.section}"]`)
    if (!el) return
    el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    el.classList.add('ring-2', 'ring-blue-400', 'rounded')
    setTimeout(() => el.classList.remove('ring-2', 'ring-blue-400'), 1500)
  },
  { immediate: true },
)

// Edits apply by themselves shortly after typing stops; an invalid state
// waits quietly (no error mid-typing) until flush() or Apply reports it.
let autoApply: ReturnType<typeof setTimeout> | null = null
function scheduleAutoApply() {
  if (autoApply) clearTimeout(autoApply)
  if (!pending.value) return
  autoApply = setTimeout(() => {
    autoApply = null
    if (pending.value && typeof build() !== 'string') apply()
  }, 500)
}
watch(formSnapshot, scheduleAutoApply)

// Unapplied edits (an invalid one, or within the last half second) also
// make the browser ask before a reload or close.
function onBeforeUnload(e: BeforeUnloadEvent) {
  if (!pending.value) return
  e.preventDefault()
  e.returnValue = ''
}
window.addEventListener('beforeunload', onBeforeUnload)
onBeforeUnmount(() => {
  if (autoApply) clearTimeout(autoApply)
  window.removeEventListener('beforeunload', onBeforeUnload)
})
</script>

<template>
  <aside v-if="node" class="absolute top-0 right-0 bottom-0 w-96 bg-white border-l shadow-lg flex flex-col">
    <header class="px-4 py-3 border-b flex justify-between items-center">
      <div>
        <div class="text-xs text-gray-400">{{ node.node_type }}</div>
        <div class="font-medium">{{ node.id }}</div>
      </div>
      <button class="text-gray-400" @click="emit('close')">&times;</button>
    </header>
    <div ref="panelBody" class="p-4 flex-1 overflow-auto space-y-3">
      <label class="flex items-center gap-2 text-sm">
        <input v-model="disabled" type="checkbox" />
        Disabled
      </label>
      <fieldset class="border rounded p-2 space-y-2">
        <legend class="text-sm text-gray-600 px-1">Settings</legend>
        <label class="flex items-center gap-2 text-sm">
          <input v-model="continueOnFail" data-testid="continue-on-fail" type="checkbox" />
          Continue on fail
        </label>
        <label class="flex items-center gap-2 text-sm">
          <input v-model="retryEnabled" data-testid="retry-enabled" type="checkbox" />
          Retry on fail
        </label>
        <div v-if="retryEnabled" class="grid grid-cols-2 gap-2 pl-6">
          <label class="text-xs text-gray-600">
            Max tries
            <input v-model="maxTries" data-testid="max-tries" type="number" min="2" max="10" class="w-full border rounded px-2 py-1 text-sm" />
          </label>
          <label class="text-xs text-gray-600">
            Wait between tries (ms)
            <input v-model="waitMs" data-testid="wait-ms" type="number" min="0" max="60000" class="w-full border rounded px-2 py-1 text-sm" />
          </label>
        </div>
        <label class="block text-xs text-gray-600">
          Timeout (ms, empty = none)
          <input v-model="timeoutMs" data-testid="timeout-ms" type="number" min="1" max="3600000" class="w-full border rounded px-2 py-1 text-sm" />
        </label>
      </fieldset>
      <AgentSettings v-if="isAgent" v-model="agentFields" :inline-tool-count="inlineToolCount" />
      <TelegramTriggerSettings v-if="isTelegramTrigger" v-model="telegramUpdates" />
      <fieldset v-if="isTelegramTrigger" class="border rounded p-2 space-y-1.5 min-w-0" data-testid="telegram-restrict">
        <legend class="text-sm text-gray-600 px-1">Only from</legend>
        <label class="block text-xs text-gray-600">
          Chat IDs (empty = any chat)
          <input v-model="telegramRestrict.chats" aria-label="Restrict to chat IDs" placeholder="123456789, -1001234567890" class="w-full border rounded px-2 py-1 text-sm font-mono" />
        </label>
        <label class="block text-xs text-gray-600">
          User IDs (empty = anyone)
          <input v-model="telegramRestrict.users" aria-label="Restrict to user IDs" placeholder="123456789" class="w-full border rounded px-2 py-1 text-sm font-mono" />
        </label>
        <p class="text-[11px] text-gray-400">Other updates are confirmed to Telegram but don't run the workflow.</p>
      </fieldset>
      <fieldset v-if="isLoop" class="border rounded p-2 space-y-1 min-w-0" data-testid="loop-settings">
        <legend class="text-sm text-gray-600 px-1">Loop Over Items</legend>
        <label class="block text-xs text-gray-600">
          Items per batch
          <input v-model="batchSize" aria-label="Items per batch" type="number" min="1" class="w-full border rounded px-2 py-1 text-sm" />
        </label>
        <p class="text-xs text-gray-500">
          The nodes on <b>loop</b> run once per batch; link the last of them back into this node. When every batch is
          done, <b>done</b> continues with everything they sent back.
        </p>
      </fieldset>
      <TelegramSendSettings v-if="isTelegramSend" v-model="telegramMessage" />
      <CodeSettings v-if="isCode" v-model="codeFields" :sources="inputSources" :node-labels="nodeLabels" />
      <WebhookSettings v-if="isWebhook" v-model="webhookForm" :workflow-id="workflowId" />
      <ScheduleSettings v-if="isSchedule" v-model="scheduleForm" />
      <WaitSettings v-if="isWait" v-model="waitForm" />
      <MergeSettings v-if="isMerge" v-model="mergeForm" />
      <SwitchSettings v-if="isSwitch" v-model="switchForm" :sources="inputSources" :node-labels="nodeLabels" />
      <HttpRequestSettings v-if="isHttp" v-model="http" :sources="inputSources" :node-labels="nodeLabels" />
      <FirecrawlSettings v-if="isFirecrawl" v-model="firecrawl" :sources="inputSources" :node-labels="nodeLabels" />
      <ConditionsEditor
        v-if="hasConditions"
        v-model="conditions"
        :sources="inputSources"
        :node-labels="nodeLabels"
        :title="node.node_type === 'core.if' ? 'Conditions' : 'Keep items that match'"
        :hint="
          node.node_type === 'core.if'
            ? 'Each item goes to the true output when the conditions hold, else to false.'
            : 'Items that don\'t match are dropped.'
        "
      />
      <SetOptionsEditor v-if="isSet" v-model="setOptions" />
      <SetFieldsEditor v-if="isSet && setOptions.mode === 'manual'" v-model="setRows" :sources="inputSources" :node-labels="nodeLabels" />
      <div v-if="takesCredential" data-testid="credential-section">
        <label class="block text-sm text-gray-600 mb-1">Credential</label>
        <CredentialPicker v-model="credentialId" :node-type="node.node_type" />
      </div>
      <details v-if="hasForm">
        <summary class="text-sm text-gray-600 cursor-pointer">Advanced (JSON)</summary>
        <textarea v-model="paramsText" rows="14" class="mt-1 w-full border rounded px-2 py-1.5 font-mono text-xs"></textarea>
      </details>
      <div v-else>
        <label class="block text-sm text-gray-600 mb-1">Parameters (JSON)</label>
        <textarea v-model="paramsText" rows="14" class="w-full border rounded px-2 py-1.5 font-mono text-xs"></textarea>
      </div>
      <!-- Outside the collapsible JSON block so agent-form errors stay visible. -->
      <p v-if="error" role="alert" class="text-sm text-red-600">{{ error }}</p>
    </div>
    <footer class="px-4 py-3 border-t">
      <div class="flex gap-2">
        <button
          data-testid="delete-node"
          class="border border-red-300 text-red-700 rounded px-3 py-2 text-sm hover:bg-red-50"
          title="Delete this node and its connections (Delete key)"
          @click="emit('delete', node.id)"
        >
          Delete
        </button>
        <button class="flex-1 bg-blue-600 text-white rounded py-2 text-sm" @click="apply">Apply</button>
      </div>
    </footer>
  </aside>
</template>
