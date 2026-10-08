<script setup lang="ts">
import { nextTick, ref } from 'vue'

/**
 * A text box for long prompts: multi-line and resizable in place, with
 * "Expand" for a large editor (the side panel is narrow).
 */
const text = defineModel<string>({ required: true })
const props = defineProps<{ label: string; rows?: number; placeholder?: string; hint?: string; code?: boolean }>()

// In a code box Tab indents (two spaces) instead of leaving the box.
function onKeydown(e: KeyboardEvent) {
  if (!props.code || e.key !== 'Tab' || e.shiftKey) return
  e.preventDefault()
  const el = e.target as HTMLTextAreaElement
  const { selectionStart: start, selectionEnd: end } = el
  text.value = text.value.slice(0, start) + '  ' + text.value.slice(end)
  el.value = text.value
  el.setSelectionRange(start + 2, start + 2)
}

const expanded = ref(false)
const bigBox = ref<HTMLTextAreaElement | null>(null)
const box = ref<HTMLTextAreaElement | null>(null)

/** Types `snippet` at the cursor (of the large editor when it's open). */
async function insert(snippet: string) {
  const el = expanded.value ? bigBox.value : box.value
  const start = el?.selectionStart ?? text.value.length
  const end = el?.selectionEnd ?? start
  text.value = text.value.slice(0, start) + snippet + text.value.slice(end)
  if (el) el.value = text.value
  await nextTick()
  el?.focus()
  el?.setSelectionRange(start + snippet.length, start + snippet.length)
}
defineExpose({ insert })

async function expand() {
  expanded.value = true
  await nextTick()
  bigBox.value?.focus()
}
</script>

<template>
  <div class="text-xs text-gray-600">
    <div class="flex justify-between items-end mb-0.5 gap-2">
      <span>{{ label }}</span>
      <span class="flex-1" />
      <slot name="actions" />
      <button
        type="button"
        :data-testid="`expand-${label}`"
        title="Open a large editor"
        class="text-blue-600 hover:underline"
        @click="expand"
      >
        ⤢ Expand
      </button>
    </div>
    <textarea
      ref="box"
      v-model="text"
      :aria-label="label"
      :rows="rows ?? 6"
      :placeholder="placeholder"
      :spellcheck="!code"
      class="w-full border rounded px-2 py-1 text-sm font-mono resize-y min-h-[4rem]"
      :class="{ 'text-xs leading-5 bg-gray-50 whitespace-pre': code }"
      @keydown="onKeydown"
    ></textarea>
    <div class="flex justify-between gap-2 text-gray-400">
      <span class="min-w-0">{{ hint }}</span>
      <span class="shrink-0 whitespace-nowrap">{{ text.length }} characters</span>
    </div>

    <div
      v-if="expanded"
      data-testid="prompt-editor"
      class="fixed inset-0 z-50 bg-black/40 flex items-center justify-center p-6"
      @keydown.esc="expanded = false"
    >
      <div class="bg-white rounded shadow-xl w-full max-w-4xl h-[85vh] flex flex-col">
        <header class="px-4 py-2 border-b flex justify-between items-center">
          <span class="text-sm font-medium text-gray-800">{{ label }}</span>
          <span class="text-xs text-gray-400">{{ text.length }} characters</span>
        </header>
        <textarea
          ref="bigBox"
          v-model="text"
          :aria-label="`${label} (large editor)`"
          :placeholder="placeholder"
          :spellcheck="!code"
          class="flex-1 m-3 border rounded p-3 text-sm font-mono resize-none"
          :class="{ 'whitespace-pre bg-gray-50': code }"
          @keydown="onKeydown"
        ></textarea>
        <footer class="px-4 py-2 border-t flex justify-between items-center">
          <span class="text-xs text-gray-400">{{ hint }}</span>
          <button
            type="button"
            data-testid="prompt-editor-done"
            class="bg-blue-600 text-white rounded px-4 py-1.5 text-sm"
            @click="expanded = false"
          >
            Done
          </button>
        </footer>
      </div>
    </div>
  </div>
</template>
