<script setup lang="ts">
import { ref, watch } from 'vue'
import type { NodeInstance, NodeSettings } from '../types/domain'
import CredentialPicker from './CredentialPicker.vue'

const props = defineProps<{ node: NodeInstance | null }>()
const emit = defineEmits<{ update: [node: NodeInstance]; close: [] }>()

const paramsText = ref('')
const error = ref('')
const disabled = ref(false)
const credentialId = ref<string | null>(null)
const continueOnFail = ref(false)
const retryEnabled = ref(false)
const maxTries = ref<number | string>(3)
const waitMs = ref<number | string>(1000)
// '' = no timeout (an empty <input type="number">).
const timeoutMs = ref<number | string>('')

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

function apply() {
  if (!props.node) return
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(paramsText.value)
  } catch {
    error.value = 'Parameters must be valid JSON.'
    return
  }
  if (credentialId.value) {
    parsed.auth = { ...((parsed.auth as object) ?? {}), credential_id: credentialId.value }
  }
  const settings = buildSettings()
  if (typeof settings === 'string') {
    error.value = settings
    return
  }
  emit('update', { ...props.node, parameters: parsed, disabled: disabled.value, settings })
  error.value = ''
}
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
    <div class="p-4 flex-1 overflow-auto space-y-3">
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
      <div>
        <label class="block text-sm text-gray-600 mb-1">Credential (for nodes that need auth)</label>
        <CredentialPicker v-model="credentialId" :node-type="node.node_type" />
      </div>
      <div>
        <label class="block text-sm text-gray-600 mb-1">Parameters (JSON)</label>
        <textarea v-model="paramsText" rows="14" class="w-full border rounded px-2 py-1.5 font-mono text-xs"></textarea>
        <p v-if="error" class="text-sm text-red-600 mt-1">{{ error }}</p>
      </div>
    </div>
    <footer class="px-4 py-3 border-t">
      <button class="w-full bg-blue-600 text-white rounded py-2 text-sm" @click="apply">Apply</button>
    </footer>
  </aside>
</template>
