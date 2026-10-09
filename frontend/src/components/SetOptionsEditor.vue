<script setup lang="ts">
import PromptBox from './PromptBox.vue'
import type { SetOptions } from '../canvas/setFields'

/** Edit Fields (Set): mode, which input fields to keep, dot notation. */
const options = defineModel<SetOptions>({ required: true })
const set = (patch: Partial<SetOptions>) => (options.value = { ...options.value, ...patch })
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="set-options">
    <legend class="text-sm text-gray-600 px-1">Edit Fields</legend>
    <label class="block text-xs text-gray-600">
      Mode
      <select :value="options.mode" aria-label="Set mode" class="w-full border rounded px-2 py-1 text-sm" @change="set({ mode: ($event.target as HTMLSelectElement).value as SetOptions['mode'] })">
        <option value="manual">Field by field</option>
        <option value="json">JSON object</option>
      </select>
    </label>
    <PromptBox
      v-if="options.mode === 'json'"
      :model-value="options.jsonOutput"
      label="JSON"
      code
      :rows="6"
      :hint="'Its fields are set on each item; values can be expressions, e.g. {&quot;id&quot;: {{ $json.id }}}.'"
      @update:model-value="(jsonOutput: string) => set({ jsonOutput })"
    />
    <label class="block text-xs text-gray-600">
      The input's other fields
      <select :value="options.include" aria-label="Include other fields" class="w-full border rounded px-2 py-1 text-sm" @change="set({ include: ($event.target as HTMLSelectElement).value as SetOptions['include'] })">
        <option value="all">Keep all of them</option>
        <option value="none">Drop them (only the fields set here)</option>
        <option value="selected">Keep only these…</option>
        <option value="except">Keep all except these…</option>
      </select>
    </label>
    <input
      v-if="options.include === 'selected' || options.include === 'except'"
      :value="options.includeFields"
      aria-label="Fields to include"
      placeholder="chat_id, message"
      class="w-full border rounded px-2 py-1 text-sm"
      @input="set({ includeFields: ($event.target as HTMLInputElement).value })"
    />
    <label class="flex items-center gap-2 text-xs">
      <input type="checkbox" aria-label="Dot notation" :checked="options.dotNotation" @change="set({ dotNotation: ($event.target as HTMLInputElement).checked })" />
      A name like <code>user.age</code> sets <code>age</code> inside <code>user</code>
    </label>
  </fieldset>
</template>
