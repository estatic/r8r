<script setup lang="ts">
import ConditionsEditor from './ConditionsEditor.vue'
import { emptySwitchRule, type SwitchForm, type SwitchRule } from '../canvas/switchRules'
import type { ConditionsForm } from '../canvas/conditions'
import type { UpstreamSource } from '../canvas/inputData'

/** Switch: an output per rule (first match wins), or an expression giving the output. */
const form = defineModel<SwitchForm>({ required: true })
defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string> }>()

const set = (patch: Partial<SwitchForm>) => (form.value = { ...form.value, ...patch })
function editRule(i: number, patch: Partial<SwitchRule>) {
  set({ rules: form.value.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) })
}
function removeRule(i: number) {
  const rules = form.value.rules.filter((_, j) => j !== i)
  // A fallback pointing at a removed or shifted output goes back to none.
  const fb = Number(form.value.fallback)
  const fallback = Number.isInteger(fb) && fb >= i ? 'none' : form.value.fallback
  set({ rules, fallback })
}
const outputLabel = (r: SwitchRule, i: number) => r.outputName.trim() || `Output ${i}`
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="switch-settings">
    <legend class="text-sm text-gray-600 px-1">Switch</legend>
    <label class="block text-xs text-gray-600">
      Mode
      <select :value="form.mode" aria-label="Switch mode" class="w-full border rounded px-2 py-1 text-sm" @change="set({ mode: ($event.target as HTMLSelectElement).value as SwitchForm['mode'] })">
        <option value="rules">Rules: an output per rule</option>
        <option value="expression">Expression: it gives the output number</option>
      </select>
    </label>

    <template v-if="form.mode === 'rules'">
      <div v-for="(rule, i) in form.rules" :key="i" class="border rounded p-1.5 space-y-1 bg-white" data-testid="switch-rule">
        <div class="flex gap-1 items-center">
          <span class="text-[10px] text-gray-500 shrink-0">Output {{ i }}</span>
          <input
            :value="rule.outputName"
            aria-label="Output name"
            placeholder="Name (optional)"
            class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs"
            @input="editRule(i, { outputName: ($event.target as HTMLInputElement).value })"
          />
          <button type="button" data-testid="remove-rule" title="Remove this rule and its output" class="px-1 text-gray-400 hover:text-red-600" @click="removeRule(i)">✕</button>
        </div>
        <ConditionsEditor
          :model-value="rule.conditions"
          :sources="sources"
          :node-labels="nodeLabels"
          :title="`Send to ${outputLabel(rule, i)} when`"
          hint=""
          @update:model-value="(conditions: ConditionsForm) => editRule(i, { conditions })"
        />
      </div>
      <button type="button" data-testid="add-rule" class="text-sm text-blue-600" @click="set({ rules: [...form.rules, emptySwitchRule()] })">+ Add routing rule</button>
      <label class="block text-xs text-gray-600">
        Items that meet no rule
        <select :value="form.fallback" aria-label="Fallback output" class="w-full border rounded px-2 py-1 text-sm" @change="set({ fallback: ($event.target as HTMLSelectElement).value })">
          <option value="none">Drop them</option>
          <option value="extra">Send to an extra "Fallback" output</option>
          <option v-for="(r, i) in form.rules" :key="i" :value="String(i)">Send to {{ outputLabel(r, i) }}</option>
        </select>
      </label>
      <label class="flex items-center gap-2 text-xs">
        <input type="checkbox" aria-label="All matching outputs" :checked="form.allMatching" @change="set({ allMatching: ($event.target as HTMLInputElement).checked })" />
        Send to every output whose rule matches (not only the first)
      </label>
    </template>

    <template v-else>
      <label class="block text-xs text-gray-600">
        Number of outputs
        <input :value="form.numberOutputs" aria-label="Number of outputs" type="number" min="1" max="32" class="w-full border rounded px-2 py-1 text-sm" @input="set({ numberOutputs: ($event.target as HTMLInputElement).value })" />
      </label>
      <label class="block text-xs text-gray-600">
        Output index (from 0)
        <input :value="form.output" aria-label="Output index" placeholder="{{ $json.priority === 'high' ? 0 : 1 }}" class="w-full border rounded px-2 py-1 text-xs font-mono" @input="set({ output: ($event.target as HTMLInputElement).value })" />
      </label>
    </template>
  </fieldset>
</template>
