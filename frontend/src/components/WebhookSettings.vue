<script setup lang="ts">
import { computed } from 'vue'
import { WEBHOOK_METHODS, type WebhookForm } from '../canvas/webhook'

/** Webhook trigger: its URL, method and how it answers the caller. */
const form = defineModel<WebhookForm>({ required: true })
const props = defineProps<{ workflowId?: string }>()
const set = (patch: Partial<WebhookForm>) => (form.value = { ...form.value, ...patch })
const url = computed(() => `${window.location.origin}/webhook-r8r/${props.workflowId ?? '<workflow id>'}/${form.value.path.trim().replace(/^\/+/, '') || '<path>'}`)
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="webhook-settings">
    <legend class="text-sm text-gray-600 px-1">Webhook</legend>
    <div class="flex gap-1">
      <select :value="form.method" aria-label="HTTP method" class="border rounded px-1 py-1 text-sm shrink-0" @change="set({ method: ($event.target as HTMLSelectElement).value })">
        <option v-for="m in WEBHOOK_METHODS" :key="m" :value="m">{{ m === 'ANY' ? 'Any method' : m }}</option>
      </select>
      <input :value="form.path" aria-label="Path" placeholder="orders or users/:id" class="flex-1 min-w-0 border rounded px-2 py-1 text-sm font-mono" @input="set({ path: ($event.target as HTMLInputElement).value })" />
    </div>
    <p class="text-[11px] text-gray-500 break-all" data-testid="webhook-url">URL: <code>{{ url }}</code></p>
    <p class="text-[11px] text-gray-400">A <code>:name</code> part captures that piece of the URL as <code>$json.params.name</code>. The run gets <code>headers</code>, <code>query</code>, <code>body</code> and <code>params</code>. It listens while the workflow is active.</p>
    <label class="block text-xs text-gray-600">
      Respond
      <select :value="form.respond" aria-label="Respond" class="w-full border rounded px-2 py-1 text-sm" @change="set({ respond: ($event.target as HTMLSelectElement).value as WebhookForm['respond'] })">
        <option value="immediately">Immediately ("Workflow was started")</option>
        <option value="lastNode">When the last node finishes, with its data</option>
        <option value="run">With the whole run record (older r8r default)</option>
      </select>
    </label>
    <label v-if="form.respond === 'lastNode'" class="block text-xs text-gray-600">
      Response data
      <select :value="form.responseData" aria-label="Response data" class="w-full border rounded px-2 py-1 text-sm" @change="set({ responseData: ($event.target as HTMLSelectElement).value as WebhookForm['responseData'] })">
        <option value="firstEntryJson">The first item</option>
        <option value="allEntries">All items (a list)</option>
        <option value="noData">No body</option>
      </select>
    </label>
    <label v-if="form.respond !== 'run'" class="block text-xs text-gray-600">
      Response code
      <input :value="form.responseCode" aria-label="Response code" type="number" min="100" max="599" class="w-full border rounded px-2 py-1 text-sm" @input="set({ responseCode: ($event.target as HTMLInputElement).value })" />
    </label>
    <p class="text-[11px] text-gray-400">To require authentication, choose a Basic Auth or API Key (Header) credential below; callers without it get 401/403.</p>
  </fieldset>
</template>
