<script setup lang="ts">
import { useToolsStore } from '../stores/tools'
import type { AgentFields } from '../types/domain'
import PromptBox from './PromptBox.vue'

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
    <label data-section="model" class="block text-xs text-gray-600">
      Model
      <input v-model="fields.model" aria-label="Model" placeholder="e.g. qwen3-coding:latest" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <PromptBox
      v-model="fields.system_prompt"
      label="System prompt"
      :rows="5"
      placeholder="Who the agent is and how it should answer…"
    />
    <PromptBox
      v-model="fields.user_message"
      label="User message"
      :rows="6"
      placeholder="{{ $json.message.text }}"
      :hint="'Expressions like {{ $json.message.text }} work here.'"
    />
    <label class="block text-xs text-gray-600">
      Max iterations
      <input v-model="fields.max_iterations" aria-label="Max iterations" type="number" min="1" max="50" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <div data-section="memory" class="text-xs text-gray-600 space-y-1 border-t pt-2">
      <label class="flex items-center gap-2 text-sm text-gray-800">
        <input v-model="fields.memory_enabled" type="checkbox" aria-label="Remember the conversation" />
        Memory: remember the conversation
      </label>
      <template v-if="fields.memory_enabled">
        <label class="block">
          Exchanges to remember
          <input
            v-model="fields.memory_window"
            aria-label="Exchanges to remember"
            type="number"
            min="1"
            max="50"
            class="w-full border rounded px-2 py-1 text-sm"
          />
        </label>
        <label class="block">
          Session key
          <input
            v-model="fields.memory_session_key"
            aria-label="Session key"
            placeholder="(empty: each Telegram chat separately)"
            class="w-full border rounded px-2 py-1 text-sm font-mono"
          />
          <span class="text-gray-400">Runs with the same key share one conversation. Expressions work here.</span>
        </label>
      </template>
    </div>
    <div data-section="tools" class="text-xs text-gray-600 space-y-1 border-t pt-2">
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