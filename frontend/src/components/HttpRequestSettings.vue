<script setup lang="ts">
import { computed, ref } from 'vue'
import FieldPicker from './FieldPicker.vue'
import PromptBox from './PromptBox.vue'
import { METHODS, type HttpForm, type Pair } from '../canvas/httpRequest'
import type { UpstreamSource } from '../canvas/inputData'

/** HTTP Request: method, URL, query parameters, headers and a JSON body. */
const form = defineModel<HttpForm>({ required: true })
defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string> }>()

const set = (patch: Partial<HttpForm>) => (form.value = { ...form.value, ...patch })
const takesBody = computed(() => !['GET', 'HEAD', 'OPTIONS'].includes(form.value.method) || form.value.bodyType !== 'none')

type ListKey = 'query' | 'headers' | 'formFields'
function editRow(key: ListKey, i: number, patch: Partial<Pair>) {
  set({ [key]: form.value[key].map((r, j) => (j === i ? { ...r, ...patch } : r)) })
}

// Where a picked field goes: the URL, or a row's value.
const picking = ref<{ key: 'url' } | { key: ListKey; row: number } | null>(null)
const isPicking = (key: string, row?: number) => picking.value?.key === key && (row === undefined || ('row' in picking.value && picking.value.row === row))
function togglePick(target: NonNullable<typeof picking.value>) {
  picking.value = picking.value && JSON.stringify(picking.value) === JSON.stringify(target) ? null : target
}
function pick(expression: string) {
  const p = picking.value
  if (!p) return
  if (p.key === 'url') set({ url: form.value.url + expression })
  else editRow(p.key, p.row, { value: expression })
  picking.value = null
}

const LISTS: { key: ListKey; title: string; add: string; namePlaceholder: string }[] = [
  { key: 'query', title: 'Query parameters', add: '+ Query parameter', namePlaceholder: 'page' },
  { key: 'headers', title: 'Headers', add: '+ Header', namePlaceholder: 'Accept' },
]
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="http-settings">
    <legend class="text-sm text-gray-600 px-1">HTTP Request</legend>
    <div class="flex gap-1">
      <select
        :value="form.method"
        aria-label="Method"
        class="border rounded px-1 py-1 text-sm shrink-0"
        @change="set({ method: ($event.target as HTMLSelectElement).value })"
      >
        <option v-for="m in METHODS" :key="m" :value="m">{{ m }}</option>
      </select>
      <input
        :value="form.url"
        aria-label="URL"
        placeholder="https://api.example.com/items/{{ $json.id }}"
        class="flex-1 min-w-0 border rounded px-2 py-1 text-sm font-mono"
        @input="set({ url: ($event.target as HTMLInputElement).value })"
      />
      <button type="button" data-testid="pick-url" class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0" @click="togglePick({ key: 'url' })">
        Pick ▾
      </button>
    </div>
    <FieldPicker v-if="isPicking('url')" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
    <p class="text-[11px] text-gray-400">Authentication: choose a Bearer Token, API Key (Header) or Basic Auth credential below.</p>

    <div v-for="list in LISTS" :key="list.key" class="border-t pt-2 space-y-1" :data-testid="`http-${list.key}`">
      <div class="text-xs text-gray-600">{{ list.title }}</div>
      <template v-for="(row, i) in form[list.key]" :key="i">
        <div class="flex gap-1">
          <input
            :value="row.name"
            aria-label="Name"
            :placeholder="list.namePlaceholder"
            class="w-1/3 min-w-0 border rounded px-1.5 py-0.5 text-xs"
            @input="editRow(list.key, i, { name: ($event.target as HTMLInputElement).value })"
          />
          <input
            :value="row.value"
            aria-label="Value"
            placeholder="value or {{ $json.x }}"
            class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs font-mono"
            @input="editRow(list.key, i, { value: ($event.target as HTMLInputElement).value })"
          />
          <button type="button" class="text-xs border rounded px-1.5 bg-white hover:bg-blue-50" title="Pick from the input" @click="togglePick({ key: list.key, row: i })">▾</button>
          <button type="button" class="px-1 text-gray-400 hover:text-red-600" title="Remove" @click="set({ [list.key]: form[list.key].filter((_, j) => j !== i) })">✕</button>
        </div>
        <FieldPicker v-if="isPicking(list.key, i)" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
      </template>
      <button type="button" class="text-xs text-blue-600" :data-testid="`add-${list.key}`" @click="set({ [list.key]: [...form[list.key], { name: '', value: '' }] })">
        {{ list.add }}
      </button>
    </div>

    <div v-if="takesBody" class="border-t pt-2 space-y-1">
      <label class="block text-xs text-gray-600">
        Body
        <select :value="form.bodyType" aria-label="Body" class="w-full border rounded px-2 py-1 text-sm" @change="set({ bodyType: ($event.target as HTMLSelectElement).value as HttpForm['bodyType'] })">
          <option value="none">No body</option>
          <option value="json">JSON</option>
          <option value="form">Form (urlencoded)</option>
          <option value="text">Plain text</option>
        </select>
      </label>
      <template v-if="form.bodyType === 'form'">
        <template v-for="(row, i) in form.formFields" :key="i">
          <div class="flex gap-1" data-testid="http-form-field">
            <input :value="row.name" aria-label="Form field name" placeholder="name" class="w-1/3 min-w-0 border rounded px-1.5 py-0.5 text-xs" @input="editRow('formFields', i, { name: ($event.target as HTMLInputElement).value })" />
            <input :value="row.value" aria-label="Form field value" placeholder="value or {{ $json.x }}" class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs font-mono" @input="editRow('formFields', i, { value: ($event.target as HTMLInputElement).value })" />
            <button type="button" class="text-xs border rounded px-1.5 bg-white hover:bg-blue-50" title="Pick from the input" @click="togglePick({ key: 'formFields', row: i })">▾</button>
            <button type="button" class="px-1 text-gray-400 hover:text-red-600" title="Remove" @click="set({ formFields: form.formFields.filter((_, j) => j !== i) })">✕</button>
          </div>
          <FieldPicker v-if="isPicking('formFields', i)" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
        </template>
        <button type="button" class="text-xs text-blue-600" data-testid="add-form-field" @click="set({ formFields: [...form.formFields, { name: '', value: '' }] })">+ Form field</button>
      </template>
      <PromptBox
        v-if="form.bodyType === 'text'"
        :model-value="form.body"
        label="Text body"
        :rows="5"
        :hint="'Sent as text/plain unless you set a Content-Type header.'"
        @update:model-value="(body: string) => set({ body })"
      />
      <PromptBox
        v-if="form.bodyType === 'json'"
        :model-value="form.body"
        label="JSON body"
        code
        :rows="6"
        :hint="'Values can be expressions inside strings, e.g. {&quot;name&quot;: &quot;{{ $json.name }}&quot;}.'"
        @update:model-value="(body: string) => set({ body })"
      />
    </div>
    <details class="border-t pt-2">
      <summary class="text-xs text-gray-600 cursor-pointer">Options</summary>
      <div class="mt-1.5 grid grid-cols-2 gap-2">
        <label class="text-xs text-gray-600">
          Timeout (ms)
          <input :value="form.timeoutMs" aria-label="Request timeout" type="number" min="1" placeholder="30000" class="w-full border rounded px-2 py-1 text-sm" @input="set({ timeoutMs: ($event.target as HTMLInputElement).value })" />
        </label>
        <label class="text-xs text-gray-600">
          Response format
          <select :value="form.responseFormat" aria-label="Response format" class="w-full border rounded px-2 py-1 text-sm" @change="set({ responseFormat: ($event.target as HTMLSelectElement).value as HttpForm['responseFormat'] })">
            <option value="auto">Autodetect</option>
            <option value="json">JSON</option>
            <option value="text">Text (under data)</option>
          </select>
        </label>
      </div>
    </details>
    <p class="text-xs text-gray-500">A JSON list of objects becomes one item each; text or HTML comes back under <code>data</code>.</p>
  </fieldset>
</template>
