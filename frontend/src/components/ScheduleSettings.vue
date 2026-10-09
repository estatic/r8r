<script setup lang="ts">
import { computed } from 'vue'
import TimezoneInput from './TimezoneInput.vue'
import { INTERVALS, WEEKDAY_NAMES, describeSchedule, type ScheduleForm } from '../canvas/schedule'

/** Schedule Trigger: when runs start. */
const form = defineModel<ScheduleForm>({ required: true })
const set = (patch: Partial<ScheduleForm>) => (form.value = { ...form.value, ...patch })
const i = computed(() => form.value.interval)
const showsEvery = computed(() => ['seconds', 'minutes', 'hours', 'days', 'months'].includes(i.value))
const showsTime = computed(() => ['days', 'weeks', 'months'].includes(i.value))
const unit = computed(() => ({ seconds: 'seconds', minutes: 'minutes', hours: 'hours', days: 'days', months: 'months' })[i.value as 'days'] ?? '')
function toggleDay(d: number, on: boolean) {
  set({ weekdays: on ? [...form.value.weekdays.filter((x) => x !== d), d] : form.value.weekdays.filter((x) => x !== d) })
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="schedule-settings">
    <legend class="text-sm text-gray-600 px-1">Schedule</legend>
    <label class="block text-xs text-gray-600">
      Run every
      <select :value="form.interval" aria-label="Interval" class="w-full border rounded px-2 py-1 text-sm" @change="set({ interval: ($event.target as HTMLSelectElement).value as ScheduleForm['interval'] })">
        <option v-for="o in INTERVALS" :key="o.value" :value="o.value">{{ o.label }}</option>
      </select>
    </label>
    <label v-if="showsEvery" class="block text-xs text-gray-600">
      Every how many {{ unit }}
      <input :value="form.every" aria-label="Every" type="number" min="1" class="w-full border rounded px-2 py-1 text-sm" @input="set({ every: ($event.target as HTMLInputElement).value })" />
    </label>
    <div v-if="i === 'weeks'" class="flex flex-wrap gap-2 text-xs text-gray-600">
      <label v-for="(name, d) in WEEKDAY_NAMES" :key="d" class="flex items-center gap-1">
        <input type="checkbox" :aria-label="name" :checked="form.weekdays.includes(d)" @change="toggleDay(d, ($event.target as HTMLInputElement).checked)" />
        {{ name }}
      </label>
    </div>
    <label v-if="i === 'months'" class="block text-xs text-gray-600">
      Day of the month
      <input :value="form.dayOfMonth" aria-label="Day of month" type="number" min="1" max="31" class="w-full border rounded px-2 py-1 text-sm" @input="set({ dayOfMonth: ($event.target as HTMLInputElement).value })" />
    </label>
    <div v-if="showsTime || i === 'hours'" class="grid grid-cols-2 gap-2">
      <label v-if="showsTime" class="text-xs text-gray-600">
        Hour (0–23)
        <input :value="form.hour" aria-label="Hour" type="number" min="0" max="23" class="w-full border rounded px-2 py-1 text-sm" @input="set({ hour: ($event.target as HTMLInputElement).value })" />
      </label>
      <label class="text-xs text-gray-600">
        Minute (0–59)
        <input :value="form.minute" aria-label="Minute" type="number" min="0" max="59" class="w-full border rounded px-2 py-1 text-sm" @input="set({ minute: ($event.target as HTMLInputElement).value })" />
      </label>
    </div>
    <label v-if="i === 'cron'" class="block text-xs text-gray-600">
      Cron expression (minute hour day month weekday)
      <input :value="form.expression" aria-label="Cron expression" placeholder="*/15 9-17 * * Mon-Fri" class="w-full border rounded px-2 py-1 text-sm font-mono" @input="set({ expression: ($event.target as HTMLInputElement).value })" />
    </label>
    <TimezoneInput :model-value="form.timezone" @update:model-value="(timezone: string) => set({ timezone })" />
    <p class="text-xs text-gray-500" data-testid="schedule-summary">{{ describeSchedule(form) }}. Runs while the workflow is active.</p>
  </fieldset>
</template>
