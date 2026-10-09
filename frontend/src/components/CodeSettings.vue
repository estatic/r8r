<script setup lang="ts">
import { computed, ref } from 'vue'
import PromptBox from './PromptBox.vue'
import { codeExample, isCodeExample, type CodeLanguage, type CodeMode } from '../canvas/codeExamples'
import { codeReference, type UpstreamSource } from '../canvas/inputData'

/** The Code node's mode, language and code. */
const model = defineModel<{ mode: CodeMode; language: CodeLanguage; script: string; writtenIn: CodeLanguage }>({ required: true })
const props = withDefaults(defineProps<{ sources?: UpstreamSource[]; nodeLabels?: Record<string, string> }>(), {
  sources: () => [],
  nodeLabels: () => ({}),
})

// "Insert field": a field reaching this node, written in the code's language.
const editor = ref<{ insert: (text: string) => void } | null>(null)
const picking = ref(false)
const search = ref('')
const hasData = computed(() => props.sources.some((s) => s.fields.length > 0))
const filteredSources = computed(() => {
  const q = search.value.toLowerCase()
  return props.sources
    .map((s) => ({ ...s, fields: s.fields.filter((f) => !q || f.path.toLowerCase().includes(q) || f.preview.toLowerCase().includes(q)) }))
    .filter((s) => s.fields.length > 0)
})
function pick(source: UpstreamSource, segments: (string | number)[]) {
  editor.value?.insert(codeReference(model.value.language, source.direct, source.nodeId, segments))
  picking.value = false
}

const isExample = isCodeExample

function setLanguage(language: CodeLanguage) {
  // Swap an untouched example; never replace code the user wrote.
  if (isExample(model.value.script)) {
    model.value = { ...model.value, language, script: codeExample(language, model.value.mode), writtenIn: language }
  } else {
    model.value = { ...model.value, language }
  }
}

function setMode(mode: CodeMode) {
  model.value = isExample(model.value.script)
    ? { ...model.value, mode, script: codeExample(model.value.language, mode) }
    : { ...model.value, mode }
}

const NAMES: Record<CodeLanguage, string> = { javaScript: 'JavaScript', python: 'Python' }
const mismatch = computed(() => model.value.language !== model.value.writtenIn && !isExample(model.value.script))
const eachItem = computed(() => model.value.mode === 'runOnceForEachItem')
const hint = computed(() => {
  const py = model.value.language === 'python'
  if (eachItem.value) {
    return py
      ? '_json: this item\'s data; _node["id"]["json"]: an earlier node\'s output. Runs once for each input item; return one dict, the item to pass on.'
      : '$json: this item\'s data; $node["id"].json: an earlier node\'s output. Runs once for each input item; return one object, the item to pass on.'
  }
  return py
    ? 'items: the input items (item.json). _json: the first item reaching this node; _node["id"]["json"]: an earlier node\'s output. Return the items to pass on.'
    : 'items: the input items (item.json). $json: the first item reaching this node; $node["id"].json: an earlier node\'s output. Return the items to pass on.'
})
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="code-settings">
    <legend class="text-sm text-gray-600 px-1">Code</legend>
    <label class="block text-xs text-gray-600">
      Mode
      <select
        :value="model.mode"
        aria-label="Mode"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="setMode(($event.target as HTMLSelectElement).value as CodeMode)"
      >
        <option value="runOnceForAllItems">Run Once for All Items</option>
        <option value="runOnceForEachItem">Run Once for Each Item</option>
      </select>
    </label>
    <label class="block text-xs text-gray-600">
      Language
      <select
        :value="model.language"
        aria-label="Language"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="setLanguage(($event.target as HTMLSelectElement).value as CodeLanguage)"
      >
        <option value="javaScript">JavaScript</option>
        <option value="python">Python</option>
      </select>
    </label>
    <p v-if="mismatch" class="text-xs text-amber-700 bg-amber-50 rounded px-2 py-1">
      This code was written in {{ NAMES[model.writtenIn] }}: rewrite it in {{ NAMES[model.language] }}, or switch back.
    </p>
    <PromptBox
      ref="editor"
      :model-value="model.script"
      label="Code"
      code
      :rows="14"
      :hint="hint"
      @update:model-value="(script: string) => (model = { ...model, script })"
    >
      <template #actions>
        <button type="button" data-testid="insert-field" class="text-blue-600 hover:underline" @click="picking = !picking; search = ''">
          Insert field ▾
        </button>
      </template>
    </PromptBox>
    <div v-if="picking" class="border rounded bg-white shadow-sm" data-testid="code-field-picker">
      <p v-if="!hasData" class="px-2 py-1.5 text-xs text-gray-500">
        {{
          sources.length === 0
            ? 'Nothing is connected to this node\'s input yet.'
            : 'Run the workflow once (or send a test message) to pick from the data reaching this node.'
        }}
      </p>
      <template v-else>
        <input v-model="search" placeholder="Search fields…" class="w-full border-b px-2 py-1 text-xs" />
        <div class="max-h-56 overflow-auto">
          <div v-for="s in filteredSources" :key="s.nodeId">
            <div class="px-2 pt-1.5 pb-0.5 text-[10px] uppercase tracking-wide text-gray-400">
              {{ nodeLabels[s.nodeId] ?? s.nodeId }}{{ s.direct ? ' — input' : '' }}
            </div>
            <button
              v-for="f in s.fields"
              :key="f.path"
              type="button"
              class="w-full text-left px-2 py-0.5 hover:bg-blue-50 flex justify-between gap-2"
              :title="codeReference(model.language, s.direct, s.nodeId, f.segments)"
              @click="pick(s, f.segments)"
            >
              <span class="font-mono text-xs text-gray-800 truncate">{{ f.path.replace(/^\./, '') }}</span>
              <span class="text-xs text-gray-400 truncate max-w-[45%]">{{ f.preview }}</span>
            </button>
          </div>
          <p v-if="filteredSources.length === 0" class="px-2 py-1.5 text-xs text-gray-400">No matches</p>
        </div>
      </template>
    </div>
  </fieldset>
</template>
