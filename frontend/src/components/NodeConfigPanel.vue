<script setup lang="ts">
import { ref, watch } from 'vue'
import type { NodeInstance } from '../types/domain'
import CredentialPicker from './CredentialPicker.vue'

const props = defineProps<{ node: NodeInstance | null }>()
const emit = defineEmits<{ update: [node: NodeInstance]; close: [] }>()

const paramsText = ref('')
const error = ref('')
const disabled = ref(false)
const credentialId = ref<string | null>(null)

watch(
  () => props.node,
  (node) => {
    if (node) {
      paramsText.value = JSON.stringify(node.parameters, null, 2)
      disabled.value = node.disabled
      error.value = ''
      const auth = node.parameters?.auth as { credential_id?: string } | undefined
      credentialId.value = auth?.credential_id ?? null
    }
  },
  { immediate: true },
)

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
  emit('update', { ...props.node, parameters: parsed, disabled: disabled.value })
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
