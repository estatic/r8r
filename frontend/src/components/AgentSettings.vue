<script setup lang="ts">
import { useToolsStore } from '../stores/tools'
import type { AgentFields } from '../types/domain'

const fields = defineModel<AgentFields>({ required: true })
defineProps<{ inlineToolCount: number }>()
const toolsStore = useToolsStore()
// Always refresh: tools may have been added in another tab (Manage tools).
toolsStore.fetchAll().catch(() => {})

function toggleTool(id: string, on: boolean) {
  const ids = new Set(fields.value.tool_ids)
  if (on) ids.add(id)
  else ids.delete(id)
  fields.value = { ...fields.value, tool_ids: [...ids] }
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2">
    <legend class="text-sm text-gray-600 px-1">AI Agent</legend>
    <label class="block text-xs text-gray-600">
      Provider
      <select v-model="fields.provider" aria-label="Provider" class="w-full border rounded px-2 py-1 text-sm">
        <option value="">(from credential)</option>
        <option value="openai">OpenAI-compatible (OpenAI, Ollama, …)</option>
        <option value="anthropic">Anthropic</option>
      </select>
    </label>
    <label class="block text-xs text-gray-600">
      Model
      <input v-model="fields.model" aria-label="Model" placeholder="e.g. qwen3-coding:latest" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <label class="block text-xs text-gray-600">
      System prompt
      <textarea v-model="fields.system_prompt" aria-label="System prompt" rows="3" class="w-full border rounded px-2 py-1 text-sm"></textarea>
    </label>
    <label class="block text-xs text-gray-600">
      User message
      <input v-model="fields.user_message" aria-label="User message" class="w-full border rounded px-2 py-1 text-sm" />
      <span class="text-gray-400">Expressions like {{ '{' + '{ $json.message.text }' + '}' }} work here.</span>
    </label>
    <label class="block text-xs text-gray-600">
      Max iterations
      <input v-model="fields.max_iterations" aria-label="Max iterations" type="number" min="1" max="50" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <div class="text-xs text-gray-600 space-y-1">
      <div class="flex justify-between"><span>Tools</span><!-- New tab: leaving the editor would drop unsaved canvas edits. -->
        <router-link to="/tools" target="_blank" data-testid="manage-tools" class="text-blue-600">Manage tools ↗</router-link></div>
      <label v-for="t in toolsStore.tools" :key="t.id" class="flex gap-2 items-start">
        <input
          type="checkbox"
          :aria-label="`Use tool ${t.name}`"
          :checked="fields.tool_ids.includes(t.id)"
          @change="toggleTool(t.id, ($event.target as HTMLInputElement).checked)"
        />
        <span><span class="font-mono">{{ t.name }}</span> — {{ t.description }}</span>
      </label>
      <p v-if="toolsStore.loaded && toolsStore.tools.length === 0" class="text-gray-400">No tools yet.</p>
      <p v-if="inlineToolCount > 0" class="text-gray-400">{{ inlineToolCount }} inline tool(s) (edit in Advanced).</p>
    </div>
  </fieldset>
</template>