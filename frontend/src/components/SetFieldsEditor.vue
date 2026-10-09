<script setup lang="ts">
import { ref } from 'vue'
import FieldPicker from './FieldPicker.vue'
import type { SetFieldRow } from '../canvas/setFields'
import type { UpstreamSource } from '../canvas/inputData'

/**
 * The Set node's fields: each gets a fixed value, or one taken from the
 * data reaching the node (picked from the run on screen, or typed).
 */
const rows = defineModel<SetFieldRow[]>({ required: true })
defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string> }>()

const pickingFor = ref<number | null>(null)

function addRow() {
  rows.value = [...rows.value, { name: '', mode: 'expression', type: 'string', text: '' }]
}

function removeRow(i: number) {
  rows.value = rows.value.filter((_, j) => j !== i)
  if (pickingFor.value === i) pickingFor.value = null
}

function update(i: number, patch: Partial<SetFieldRow>) {
  rows.value = rows.value.map((r, j) => (j === i ? { ...r, ...patch } : r))
}

function setMode(i: number, mode: SetFieldRow['mode']) {
  update(i, mode === 'fixed' ? { mode, type: 'string', text: '' } : { mode, text: '' })
}

function setType(i: number, type: SetFieldRow['type']) {
  update(i, { type, text: type === 'boolean' ? 'true' : '' })
}

function togglePicker(i: number) {
  pickingFor.value = pickingFor.value === i ? null : i
}

function pick(i: number, expression: string, path: string) {
  const row = rows.value[i]
  // A new field takes its name from what was picked: `.message.chat.id` -> "id".
  const name = row.name.trim() || (path.match(/([A-Za-z_$][\w$]*)[^A-Za-z_$\w]*$/)?.[1] ?? '')
  update(i, { mode: 'expression', type: 'string', text: expression, name })
  pickingFor.value = null
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="set-fields">
    <legend class="text-sm text-gray-600 px-1">Fields to set</legend>
    <p class="text-xs text-gray-500">Each field is added to the item passed to the next node.</p>
    <p v-if="rows.length === 0" class="text-xs text-gray-400">No fields yet.</p>

    <div v-for="(row, i) in rows" :key="i" class="border rounded p-2 space-y-1.5 bg-gray-50" data-testid="set-field-row">
      <div class="flex gap-1">
        <input
          :value="row.name"
          data-testid="field-name"
          placeholder="Field name"
          class="flex-1 min-w-0 border rounded px-2 py-1 text-sm"
          @input="update(i, { name: ($event.target as HTMLInputElement).value })"
        />
        <div class="flex text-xs border rounded overflow-hidden shrink-0">
          <button
            type="button"
            data-testid="mode-expression"
            class="px-2"
            :class="row.mode === 'expression' ? 'bg-blue-600 text-white' : 'bg-white text-gray-600'"
            @click="setMode(i, 'expression')"
          >
            From input
          </button>
          <button
            type="button"
            data-testid="mode-fixed"
            class="px-2"
            :class="row.mode === 'fixed' ? 'bg-blue-600 text-white' : 'bg-white text-gray-600'"
            @click="setMode(i, 'fixed')"
          >
            Fixed
          </button>
        </div>
        <button type="button" data-testid="remove-field" title="Remove this field" class="px-1.5 text-gray-400 hover:text-red-600" @click="removeRow(i)">
          ✕
        </button>
      </div>

      <template v-if="row.mode === 'expression'">
        <div class="flex gap-1">
          <input
            :value="row.text"
            data-testid="field-expression"
            placeholder="{{ $json.message.chat.id }}"
            class="flex-1 min-w-0 border rounded px-2 py-1 text-xs font-mono"
            @input="update(i, { text: ($event.target as HTMLInputElement).value })"
          />
          <button
            type="button"
            data-testid="pick-field"
            class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0"
            @click="togglePicker(i)"
          >
            Pick ▾
          </button>
        </div>
        <FieldPicker v-if="pickingFor === i" :sources="sources" :node-labels="nodeLabels" @pick="(expression, path) => pick(i, expression, path)" />
      </template>

      <div v-else class="flex gap-1">
        <select
          :value="row.type"
          data-testid="field-type"
          class="border rounded px-1 py-1 text-xs shrink-0"
          @change="setType(i, ($event.target as HTMLSelectElement).value as SetFieldRow['type'])"
        >
          <option value="string">Text</option>
          <option value="number">Number</option>
          <option value="boolean">True/false</option>
          <option value="json">JSON</option>
        </select>
        <select
          v-if="row.type === 'boolean'"
          :value="row.text"
          data-testid="field-value"
          class="flex-1 border rounded px-2 py-1 text-sm"
          @change="update(i, { text: ($event.target as HTMLSelectElement).value })"
        >
          <option value="true">true</option>
          <option value="false">false</option>
        </select>
        <input
          v-else
          :value="row.text"
          data-testid="field-value"
          :type="row.type === 'number' ? 'number' : 'text'"
          :placeholder="row.type === 'json' ? '{&quot;key&quot;: &quot;value&quot;}' : 'Value'"
          class="flex-1 min-w-0 border rounded px-2 py-1 text-sm"
          :class="{ 'font-mono text-xs': row.type === 'json' }"
          @input="update(i, { text: ($event.target as HTMLInputElement).value })"
        />
      </div>
    </div>

    <button type="button" data-testid="add-field" class="text-sm text-blue-600" @click="addRow">+ Add field</button>
  </fieldset>
</template>
