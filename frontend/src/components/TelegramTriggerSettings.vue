<script setup lang="ts">
/** "Trigger on" for the Telegram Trigger: n8n's update choices. */
const TELEGRAM_UPDATES: { value: string; label: string }[] = [
  { value: 'callback_query', label: 'On callback query' },
  { value: 'channel_post', label: 'On channel post' },
  { value: 'edited_channel_post', label: 'On edited channel post' },
  { value: 'edited_message', label: 'On edited message' },
  { value: 'inline_query', label: 'On inline query' },
  { value: 'message', label: 'On message' },
  { value: 'poll', label: 'On poll change' },
  { value: 'pre_checkout_query', label: 'On pre-checkout query' },
]

const model = defineModel<string[]>({ required: true })

function toggle(value: string, on: boolean) {
  if (value === '*') {
    model.value = on ? ['*'] : []
    return
  }
  const rest = model.value.filter((v) => v !== '*' && v !== value)
  model.value = on ? [...rest, value] : rest
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-1">
    <legend class="text-sm text-gray-600 px-1">Trigger on</legend>
    <label class="flex items-center gap-2 text-sm font-medium">
      <input
        data-testid="tg-update-*"
        type="checkbox"
        :checked="model.includes('*')"
        @change="toggle('*', ($event.target as HTMLInputElement).checked)"
      />
      All updates
    </label>
    <label v-for="u in TELEGRAM_UPDATES" :key="u.value" class="flex items-center gap-2 text-sm pl-4">
      <input
        :data-testid="`tg-update-${u.value}`"
        type="checkbox"
        :checked="model.includes(u.value)"
        @change="toggle(u.value, ($event.target as HTMLInputElement).checked)"
      />
      {{ u.label }}
    </label>
  </fieldset>
</template>
