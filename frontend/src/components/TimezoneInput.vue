<script setup lang="ts">
/** A time zone name (Europe/Berlin), with the browser's list as suggestions; empty = the server's default. */
const model = defineModel<string>({ required: true })
const zones: string[] = (() => {
  try {
    return (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? []
  } catch {
    return []
  }
})()
const listId = `tz-${Math.random().toString(36).slice(2)}`
</script>

<template>
  <label class="block text-xs text-gray-600">
    Time zone
    <input v-model="model" aria-label="Time zone" :list="listId" placeholder="Server default (GENERIC_TIMEZONE, else UTC)" class="w-full border rounded px-2 py-1 text-sm" />
    <datalist :id="listId">
      <option v-for="z in zones" :key="z" :value="z" />
    </datalist>
  </label>
</template>
