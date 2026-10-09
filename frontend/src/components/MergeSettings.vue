<script setup lang="ts">
import { JOIN_MODES, type FieldPair, type MergeForm } from '../canvas/merge'

/** Merge: how the items of input 1 and input 2 come together. */
const form = defineModel<MergeForm>({ required: true })
const set = (patch: Partial<MergeForm>) => (form.value = { ...form.value, ...patch })
function editPair(i: number, patch: Partial<FieldPair>) {
  set({ matchFields: form.value.matchFields.map((m, j) => (j === i ? { ...m, ...patch } : m)) })
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="merge-settings">
    <legend class="text-sm text-gray-600 px-1">Merge</legend>
    <label class="block text-xs text-gray-600">
      Mode
      <select :value="form.mode" aria-label="Merge mode" class="w-full border rounded px-2 py-1 text-sm" @change="set({ mode: ($event.target as HTMLSelectElement).value as MergeForm['mode'] })">
        <option value="append">Append: input 1's items, then input 2's</option>
        <option value="combine">Combine: join items of both inputs</option>
        <option value="chooseBranch">Choose branch: pass on one input's items</option>
      </select>
    </label>

    <template v-if="form.mode === 'combine'">
      <label class="block text-xs text-gray-600">
        Combine by
        <select :value="form.combineBy" aria-label="Combine by" class="w-full border rounded px-2 py-1 text-sm" @change="set({ combineBy: ($event.target as HTMLSelectElement).value as MergeForm['combineBy'] })">
          <option value="matchingFields">Matching fields</option>
          <option value="position">Position (1st with 1st, …)</option>
          <option value="allCombinations">All possible combinations</option>
        </select>
      </label>
      <template v-if="form.combineBy === 'matchingFields'">
        <div class="text-xs text-gray-600">Fields to match (<code>a.b</code> reaches into objects)</div>
        <div v-for="(m, i) in form.matchFields" :key="i" class="flex gap-1 items-center" data-testid="match-pair">
          <input :value="m.field1" aria-label="Input 1 field" placeholder="Input 1 field, e.g. id" class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs" @input="editPair(i, { field1: ($event.target as HTMLInputElement).value })" />
          <span class="text-xs text-gray-400">=</span>
          <input :value="m.field2" aria-label="Input 2 field" placeholder="Input 2 field, e.g. user_id" class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs" @input="editPair(i, { field2: ($event.target as HTMLInputElement).value })" />
          <button v-if="form.matchFields.length > 1" type="button" class="px-1 text-gray-400 hover:text-red-600" title="Remove" @click="set({ matchFields: form.matchFields.filter((_, j) => j !== i) })">✕</button>
        </div>
        <button type="button" class="text-xs text-blue-600" @click="set({ matchFields: [...form.matchFields, { field1: '', field2: '' }] })">+ Field to match</button>
        <label class="block text-xs text-gray-600">
          Output
          <select :value="form.joinMode" aria-label="Join mode" class="w-full border rounded px-2 py-1 text-sm" @change="set({ joinMode: ($event.target as HTMLSelectElement).value })">
            <option v-for="j in JOIN_MODES" :key="j.value" :value="j.value">{{ j.label }}</option>
          </select>
        </label>
      </template>
      <label v-if="form.combineBy === 'position'" class="flex items-center gap-2 text-xs">
        <input type="checkbox" aria-label="Include unpaired" :checked="form.includeUnpaired" @change="set({ includeUnpaired: ($event.target as HTMLInputElement).checked })" />
        Keep items left over when one input has more
      </label>
      <label class="block text-xs text-gray-600">
        When both have a field
        <select :value="form.clash" aria-label="Clash" class="w-full border rounded px-2 py-1 text-sm" @change="set({ clash: ($event.target as HTMLSelectElement).value as MergeForm['clash'] })">
          <option value="preferInput2">Input 2's value wins</option>
          <option value="preferInput1">Input 1's value wins</option>
        </select>
      </label>
    </template>

    <label v-if="form.mode === 'chooseBranch'" class="block text-xs text-gray-600">
      Pass on
      <select :value="form.outputInput" aria-label="Output input" class="w-full border rounded px-2 py-1 text-sm" @change="set({ outputInput: ($event.target as HTMLSelectElement).value as MergeForm['outputInput'] })">
        <option value="input1">Input 1's items</option>
        <option value="input2">Input 2's items</option>
      </select>
    </label>
    <p class="text-xs text-gray-500">Connect the two branches to <b>Input 1</b> and <b>Input 2</b> on the node's left side.</p>
  </fieldset>
</template>
