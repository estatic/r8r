<script setup lang="ts">
import TimezoneInput from './TimezoneInput.vue'
import type { WaitForm } from '../canvas/schedule'

/** Wait: for a while, or until a time. */
const form = defineModel<WaitForm>({ required: true })
const set = (patch: Partial<WaitForm>) => (form.value = { ...form.value, ...patch })
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="wait-settings">
    <legend class="text-sm text-gray-600 px-1">Wait</legend>
    <label class="block text-xs text-gray-600">
      Resume
      <select :value="form.resume" aria-label="Resume" class="w-full border rounded px-2 py-1 text-sm" @change="set({ resume: ($event.target as HTMLSelectElement).value as WaitForm['resume'] })">
        <option value="interval">After a time interval</option>
        <option value="at">At a specific time</option>
      </select>
    </label>
    <div v-if="form.resume === 'interval'" class="flex gap-1">
      <input :value="form.amount" aria-label="Amount" placeholder="5 or {{ $json.delay }}" class="flex-1 min-w-0 border rounded px-2 py-1 text-sm" @input="set({ amount: ($event.target as HTMLInputElement).value })" />
      <select :value="form.unit" aria-label="Unit" class="border rounded px-2 py-1 text-sm" @change="set({ unit: ($event.target as HTMLSelectElement).value as WaitForm['unit'] })">
        <option value="seconds">seconds</option>
        <option value="minutes">minutes</option>
        <option value="hours">hours</option>
        <option value="days">days</option>
      </select>
    </div>
    <template v-else>
      <label class="block text-xs text-gray-600">
        Date and time
        <input :value="form.dateTime" aria-label="Date and time" placeholder="2026-10-09T18:30 or {{ $json.remind_at }}" class="w-full border rounded px-2 py-1 text-sm font-mono" @input="set({ dateTime: ($event.target as HTMLInputElement).value })" />
      </label>
      <TimezoneInput :model-value="form.timezone" @update:model-value="(timezone: string) => set({ timezone })" />
    </template>
    <p class="text-xs text-gray-500">The run waits in memory (up to 31 days); restarting r8r ends it. A time already past continues at once.</p>
  </fieldset>
</template>
