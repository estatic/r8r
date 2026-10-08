<script setup lang="ts">
import PromptBox from './PromptBox.vue'
import type { InlineButton, Markup, MessageForm } from '../canvas/telegramMessage'

/** Telegram Send Message: chat, text, formatting, reply markup, additional fields. */
const form = defineModel<MessageForm>({ required: true })

const set = (patch: Partial<MessageForm>) => (form.value = { ...form.value, ...patch })
const setMarkup = (markup: Markup) => set({ markup })

function setKind(kind: Markup['kind']) {
  switch (kind) {
    case 'inline':
      return setMarkup({ kind, rows: [[{ text: '', kind: 'callback', value: '' }]] })
    case 'keyboard':
      return setMarkup({ kind, rows: [['']], resize: true, oneTime: false, placeholder: '' })
    case 'forceReply':
      return setMarkup({ kind, placeholder: '' })
    default:
      return setMarkup({ kind } as Markup)
  }
}

// Inline keyboard: rows of buttons.
function inlineRows(): InlineButton[][] {
  return form.value.markup.kind === 'inline' ? form.value.markup.rows : []
}
function setInline(rows: InlineButton[][]) {
  setMarkup({ kind: 'inline', rows })
}
function editInline(r: number, b: number, patch: Partial<InlineButton>) {
  setInline(inlineRows().map((row, i) => (i !== r ? row : row.map((btn, j) => (j === b ? { ...btn, ...patch } : btn)))))
}

// Reply keyboard: rows of labels.
function keyboard() {
  return form.value.markup.kind === 'keyboard' ? form.value.markup : null
}
function setKeyboard(patch: Partial<Extract<Markup, { kind: 'keyboard' }>>) {
  const k = keyboard()
  if (k) setMarkup({ ...k, ...patch })
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="telegram-send-settings">
    <legend class="text-sm text-gray-600 px-1">Send Message</legend>

    <label class="block text-xs text-gray-600">
      <span class="flex justify-between">
        <span>Chat ID</span>
        <button type="button" class="text-blue-600 hover:underline" @click="set({ chatId: '{{ $json.message.chat.id }}' })">
          reply to the incoming chat
        </button>
      </span>
      <input
        :value="form.chatId"
        aria-label="Chat ID"
        placeholder="{{ $json.message.chat.id }} or 123456789 or @channel"
        class="w-full border rounded px-2 py-1 text-sm font-mono"
        @input="set({ chatId: ($event.target as HTMLInputElement).value })"
      />
    </label>

    <PromptBox
      :model-value="form.text"
      label="Text"
      :rows="5"
      :hint="'Expressions like {{ $json.output }} work here.'"
      @update:model-value="(text: string) => set({ text })"
    />

    <label class="block text-xs text-gray-600">
      Parse mode
      <select :value="form.parseMode" aria-label="Parse mode" class="w-full border rounded px-2 py-1 text-sm" @change="set({ parseMode: ($event.target as HTMLSelectElement).value as MessageForm['parseMode'] })">
        <option value="">Plain text</option>
        <option value="HTML">HTML</option>
        <option value="MarkdownV2">MarkdownV2</option>
        <option value="Markdown">Markdown (legacy)</option>
      </select>
    </label>

    <div class="border-t pt-2 space-y-1.5">
      <label class="block text-xs text-gray-600">
        Reply markup
        <select :value="form.markup.kind" aria-label="Reply markup" class="w-full border rounded px-2 py-1 text-sm" @change="setKind(($event.target as HTMLSelectElement).value as Markup['kind'])">
          <option value="none">None</option>
          <option value="inline">Inline keyboard (buttons under the message)</option>
          <option value="keyboard">Reply keyboard (replaces the user's keyboard)</option>
          <option value="remove">Remove the reply keyboard</option>
          <option value="forceReply">Force reply</option>
        </select>
      </label>

      <!-- Inline keyboard -->
      <div v-if="form.markup.kind === 'inline'" class="space-y-1.5" data-testid="inline-keyboard">
        <div v-for="(row, r) in form.markup.rows" :key="r" class="border rounded p-1.5 bg-gray-50 space-y-1">
          <div class="flex justify-between text-[10px] text-gray-500">
            <span>Row {{ r + 1 }}</span>
            <button type="button" class="hover:text-red-600" title="Remove this row" @click="setInline(inlineRows().filter((_, i) => i !== r))">✕ row</button>
          </div>
          <div v-for="(btn, b) in row" :key="b" class="flex gap-1">
            <input :value="btn.text" aria-label="Button label" placeholder="Label" class="w-1/3 min-w-0 border rounded px-1.5 py-0.5 text-xs" @input="editInline(r, b, { text: ($event.target as HTMLInputElement).value })" />
            <select :value="btn.kind" aria-label="Button action" class="border rounded px-1 py-0.5 text-xs" @change="editInline(r, b, { kind: ($event.target as HTMLSelectElement).value as InlineButton['kind'] })">
              <option value="callback">Callback</option>
              <option value="url">URL</option>
            </select>
            <input
              :value="btn.value"
              aria-label="Button value"
              :placeholder="btn.kind === 'url' ? 'https://…' : 'data sent back'"
              class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs font-mono"
              @input="editInline(r, b, { value: ($event.target as HTMLInputElement).value })"
            />
            <button type="button" class="px-1 text-gray-400 hover:text-red-600" title="Remove this button" @click="setInline(inlineRows().map((rw, i) => (i === r ? rw.filter((_, j) => j !== b) : rw)))">✕</button>
          </div>
          <button type="button" class="text-xs text-blue-600" data-testid="add-button" @click="setInline(inlineRows().map((rw, i) => (i === r ? [...rw, { text: '', kind: 'callback', value: '' }] : rw)))">+ Button</button>
        </div>
        <button type="button" class="text-xs text-blue-600" data-testid="add-row" @click="setInline([...inlineRows(), [{ text: '', kind: 'callback', value: '' }]])">+ Row</button>
      </div>

      <!-- Reply keyboard -->
      <div v-if="keyboard()" class="space-y-1.5" data-testid="reply-keyboard">
        <div v-for="(row, r) in keyboard()!.rows" :key="r" class="flex gap-1 items-center">
          <span class="text-[10px] text-gray-500 w-10">Row {{ r + 1 }}</span>
          <input
            v-for="(label, b) in row"
            :key="b"
            :value="label"
            aria-label="Key label"
            placeholder="Key"
            class="flex-1 min-w-0 border rounded px-1.5 py-0.5 text-xs"
            @input="setKeyboard({ rows: keyboard()!.rows.map((rw, i) => (i === r ? rw.map((l, j) => (j === b ? ($event.target as HTMLInputElement).value : l)) : rw)) })"
          />
          <button type="button" class="text-xs text-blue-600" title="Add a key to this row" @click="setKeyboard({ rows: keyboard()!.rows.map((rw, i) => (i === r ? [...rw, ''] : rw)) })">+</button>
          <button type="button" class="text-xs text-gray-400 hover:text-red-600" title="Remove this row" @click="setKeyboard({ rows: keyboard()!.rows.filter((_, i) => i !== r) })">✕</button>
        </div>
        <button type="button" class="text-xs text-blue-600" @click="setKeyboard({ rows: [...keyboard()!.rows, ['']] })">+ Row</button>
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" :checked="keyboard()!.resize" @change="setKeyboard({ resize: ($event.target as HTMLInputElement).checked })" /> Fit the keyboard to its keys</label>
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" :checked="keyboard()!.oneTime" @change="setKeyboard({ oneTime: ($event.target as HTMLInputElement).checked })" /> Hide it after one use</label>
        <input :value="keyboard()!.placeholder" placeholder="Input field placeholder (optional)" class="w-full border rounded px-2 py-1 text-xs" @input="setKeyboard({ placeholder: ($event.target as HTMLInputElement).value })" />
      </div>

      <input
        v-if="form.markup.kind === 'forceReply'"
        :value="form.markup.placeholder"
        aria-label="Force reply placeholder"
        placeholder="Input field placeholder (optional)"
        class="w-full border rounded px-2 py-1 text-xs"
        @input="setMarkup({ kind: 'forceReply', placeholder: ($event.target as HTMLInputElement).value })"
      />
    </div>

    <details class="border-t pt-2" data-testid="additional-fields">
      <summary class="text-xs text-gray-600 cursor-pointer">Additional fields</summary>
      <div class="mt-1.5 space-y-1.5">
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" aria-label="Disable notification" :checked="form.disableNotification" @change="set({ disableNotification: ($event.target as HTMLInputElement).checked })" /> Send silently (no notification)</label>
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" aria-label="Protect content" :checked="form.protectContent" @change="set({ protectContent: ($event.target as HTMLInputElement).checked })" /> Protect from forwarding and saving</label>
        <label class="flex items-center gap-2 text-xs"><input type="checkbox" aria-label="Disable link preview" :checked="form.disableLinkPreview" @change="set({ disableLinkPreview: ($event.target as HTMLInputElement).checked })" /> Disable link preview</label>
        <label class="block text-xs text-gray-600">
          Reply to message ID
          <input :value="form.replyToMessageId" aria-label="Reply to message ID" placeholder="{{ $json.message.message_id }}" class="w-full border rounded px-2 py-1 text-xs font-mono" @input="set({ replyToMessageId: ($event.target as HTMLInputElement).value })" />
        </label>
        <label class="block text-xs text-gray-600">
          Message thread ID (forum topic)
          <input :value="form.messageThreadId" aria-label="Message thread ID" class="w-full border rounded px-2 py-1 text-xs font-mono" @input="set({ messageThreadId: ($event.target as HTMLInputElement).value })" />
        </label>
      </div>
    </details>
  </fieldset>
</template>
