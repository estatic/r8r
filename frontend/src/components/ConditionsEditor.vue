<script setup lang="ts">
import { ref } from 'vue'
import FieldPicker from './FieldPicker.vue'
import { OPERATORS, emptyRule, isUnary, type ConditionRule, type ConditionsForm } from '../canvas/conditions'
import type { UpstreamSource } from '../canvas/inputData'

/** If / Filter: rules on each item's data, joined by AND or OR. */
const form = defineModel<ConditionsForm>({ required: true })
defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string>; title: string; hint: string }>()

/** Which side of which rule the field list is open for. */
const picking = ref<{ rule: number; side: 'left' | 'right' } | null>(null)

function update(i: number, patch: Partial<ConditionRule>) {
  form.value = { ...form.value, rules: form.value.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }
}
function remove(i: number) {
  form.value = { ...form.value, rules: form.value.rules.filter((_, j) => j !== i) }
  picking.value = null
}
function add() {
  form.value = { ...form.value, rules: [...form.value.rules, emptyRule()] }
}
function togglePicker(rule: number, side: 'left' | 'right') {
  picking.value = picking.value?.rule === rule && picking.value.side === side ? null : { rule, side }
}
function pick(expression: string) {
  if (!picking.value) return
  update(picking.value.rule, { [picking.value.side]: expression })
  picking.value = null
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="conditions">
    <legend class="text-sm text-gray-600 px-1">{{ title }}</legend>
    <p class="text-xs text-gray-500">{{ hint }}</p>

    <template v-for="(rule, i) in form.rules" :key="i">
      <div v-if="i > 0" class="text-[10px] uppercase tracking-wide text-gray-500 text-center">{{ form.combinator }}</div>
      <div class="border rounded p-2 space-y-1.5 bg-gray-50" data-testid="condition-row">
        <div class="flex gap-1">
          <input
            :value="rule.left"
            aria-label="Value to check"
            placeholder="{{ $json.status }}"
            class="flex-1 min-w-0 border rounded px-2 py-1 text-xs font-mono"
            @input="update(i, { left: ($event.target as HTMLInputElement).value })"
          />
          <button type="button" data-testid="pick-left" class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0" @click="togglePicker(i, 'left')">
            Pick ▾
          </button>
          <button type="button" data-testid="remove-condition" title="Remove this condition" class="px-1.5 text-gray-400 hover:text-red-600" @click="remove(i)">
            ✕
          </button>
        </div>
        <FieldPicker v-if="picking?.rule === i && picking.side === 'left'" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
        <select
          :value="rule.operator"
          aria-label="Operator"
          class="w-full border rounded px-2 py-1 text-sm"
          @change="update(i, { operator: ($event.target as HTMLSelectElement).value })"
        >
          <option v-for="op in OPERATORS" :key="op.value" :value="op.value">{{ op.label }}</option>
        </select>
        <template v-if="!isUnary(rule.operator)">
          <div class="flex gap-1">
            <input
              :value="rule.right"
              aria-label="Value to compare to"
              placeholder="Value, or {{ $json.other }}"
              class="flex-1 min-w-0 border rounded px-2 py-1 text-xs font-mono"
              @input="update(i, { right: ($event.target as HTMLInputElement).value })"
            />
            <button type="button" data-testid="pick-right" class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0" @click="togglePicker(i, 'right')">
              Pick ▾
            </button>
          </div>
          <FieldPicker v-if="picking?.rule === i && picking.side === 'right'" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
        </template>
      </div>
    </template>

    <div class="flex items-center justify-between gap-2">
      <button type="button" data-testid="add-condition" class="text-sm text-blue-600" @click="add">+ Add condition</button>
      <label v-if="form.rules.length > 1" class="text-xs text-gray-600 flex items-center gap-1">
        Match
        <select
          :value="form.combinator"
          aria-label="Combine conditions"
          class="border rounded px-1 py-0.5 text-xs"
          @change="form = { ...form, combinator: ($event.target as HTMLSelectElement).value as ConditionsForm['combinator'] }"
        >
          <option value="and">all of them (AND)</option>
          <option value="or">any of them (OR)</option>
        </select>
      </label>
    </div>
  </fieldset>
</template>
