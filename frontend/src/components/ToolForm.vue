<script setup lang="ts">
import { ref, watch } from 'vue'
import { ApiError } from '../api/client'
import { useToolsStore } from '../stores/tools'
import { useCredentialsStore } from '../stores/credentials'
import CredentialPicker from './CredentialPicker.vue'
import {
  PARAMETER_TEMPLATES,
  TOOL_NODE_TYPES,
  argsToSchema,
  authForCredentialType,
  schemaToArgs,
  type ToolArgument,
} from '../tools/schema'
import type { Tool, ToolArgumentType } from '../types/domain'

const props = defineProps<{ toolId?: string }>()
const emit = defineEmits<{ saved: [tool: Tool]; cancel: [] }>()

const store = useToolsStore()
const credentialsStore = useCredentialsStore()
const editing = !!props.toolId

const name = ref('')
const description = ref('')
const nodeType = ref('core.httpRequest')
const credentialId = ref<string | null>(null)
const args = ref<ToolArgument[]>([])
const paramsText = ref(JSON.stringify(PARAMETER_TEMPLATES['core.httpRequest'], null, 2))
const error = ref('')

const ARG_TYPES: ToolArgumentType[] = ['string', 'number', 'integer', 'boolean']
const NAME_PATTERN = /^[A-Za-z0-9_-]{1,64}$/
const ARG_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/

// A new tool starts from its node type's template; switching the type on a
// new tool swaps the template. An existing tool's parameters are never
// replaced behind the user's back.
watch(nodeType, (type) => {
  if (!editing) paramsText.value = JSON.stringify(PARAMETER_TEMPLATES[type] ?? {}, null, 2)
})

if (editing) {
  store
    .get(props.toolId!)
    .then((tool) => {
      name.value = tool.name
      description.value = tool.description
      nodeType.value = tool.node_type
      args.value = schemaToArgs(tool.argument_schema)
      const { auth, ...rest } = tool.parameters as { auth?: { credential_id?: string } } & Record<string, unknown>
      credentialId.value = auth?.credential_id ?? null
      paramsText.value = JSON.stringify(rest, null, 2)
    })
    .catch(() => {
      error.value = 'Failed to load tool.'
    })
}

function addArgument() {
  args.value = [...args.value, { name: '', type: 'string', description: '', required: false }]
}

function removeArgument(index: number) {
  args.value = args.value.filter((_, i) => i !== index)
}

function validationError(): string | null {
  if (!NAME_PATTERN.test(name.value.trim())) return 'Name must be 1-64 letters, digits, _ or -.'
  const seen = new Set<string>()
  for (const a of args.value) {
    if (!ARG_NAME_PATTERN.test(a.name)) return `Argument name "${a.name}" must start with a letter or _ and contain only letters, digits or _.`
    if (seen.has(a.name)) return `Argument "${a.name}" is declared twice.`
    seen.add(a.name)
  }
  return null
}

async function save() {
  error.value = validationError() ?? ''
  if (error.value) return
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(paramsText.value)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('not an object')
  } catch {
    error.value = 'Parameters must be a JSON object.'
    return
  }
  const credentialType = credentialsStore.credentials.find((c) => c.id === credentialId.value)?.credential_type
  const body = {
    name: name.value.trim(),
    description: description.value,
    node_type: nodeType.value,
    argument_schema: argsToSchema(args.value),
    parameters: {
      ...parsed,
      ...(credentialId.value ? { auth: authForCredentialType(nodeType.value, credentialId.value, credentialType) } : {}),
    },
  }
  try {
    emit('saved', editing ? await store.update(props.toolId!, body) : await store.create(body))
  } catch (e) {
    error.value = e instanceof ApiError && e.message ? e.message : 'Failed to save tool.'
  }
}
</script>

<template>
  <div class="border rounded p-3 space-y-3 bg-gray-50">
    <label class="block text-xs text-gray-600">
      Name
      <input v-model="name" aria-label="Name" autocomplete="off" placeholder="e.g. web_search" class="w-full border rounded px-2 py-1 text-sm font-mono" />
    </label>
    <label class="block text-xs text-gray-600">
      Description
      <textarea v-model="description" aria-label="Description" rows="2" class="w-full border rounded px-2 py-1 text-sm"></textarea>
      <span class="text-gray-400">The model reads this to decide when to call the tool.</span>
    </label>
    <label class="block text-xs text-gray-600">
      Node type
      <select v-model="nodeType" aria-label="Node type" class="w-full border rounded px-2 py-1 text-sm">
        <option v-for="t in TOOL_NODE_TYPES" :key="t.value" :value="t.value">{{ t.label }}</option>
      </select>
    </label>
    <div class="text-xs text-gray-600">
      Credential
      <CredentialPicker v-model="credentialId" :node-type="nodeType" />
    </div>

    <div class="space-y-1">
      <div class="text-xs text-gray-600">Arguments the model fills in</div>
      <div v-for="(a, i) in args" :key="i" class="grid grid-cols-12 gap-1 items-center">
        <input v-model="a.name" aria-label="Argument name" placeholder="name" class="col-span-3 border rounded px-1 py-1 text-xs font-mono" />
        <select v-model="a.type" aria-label="Argument type" class="col-span-2 border rounded px-1 py-1 text-xs">
          <option v-for="t in ARG_TYPES" :key="t" :value="t">{{ t }}</option>
        </select>
        <input v-model="a.description" aria-label="Argument description" placeholder="what it is" class="col-span-4 border rounded px-1 py-1 text-xs" />
        <label class="col-span-2 flex items-center gap-1 text-xs">
          <input v-model="a.required" aria-label="Argument required" type="checkbox" /> required
        </label>
        <button type="button" class="col-span-1 text-xs text-red-600" :aria-label="`Remove argument ${a.name}`" @click="removeArgument(i)">✕</button>
      </div>
      <button type="button" data-testid="add-argument" class="text-xs text-blue-600" @click="addArgument">+ Add argument</button>
    </div>

    <label class="block text-xs text-gray-600">
      Parameters (JSON)
      <textarea v-model="paramsText" aria-label="Parameters (JSON)" rows="6" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
      <span v-if="nodeType === 'core.code'" class="text-gray-400">The script reads the arguments as <code>$args</code> (e.g. <code>$args.n</code>); they are data, never inserted into the code.</span>
      <span v-else class="text-gray-400">Use {{ '{' + '{ $args.<name> }' + '}' }} to insert an argument. In URLs keep the scheme and host fixed and wrap values in encodeURIComponent(...).</span>
    </label>

    <div class="flex gap-2">
      <button type="button" data-testid="save-tool" class="text-xs bg-blue-600 text-white rounded px-3 py-1" @click="save">{{ editing ? 'Save' : 'Create' }}</button>
      <button type="button" class="text-xs text-gray-600" @click="emit('cancel')">Cancel</button>
    </div>
    <p v-if="error" role="alert" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>
