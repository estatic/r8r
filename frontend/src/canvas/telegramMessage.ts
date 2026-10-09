/** Telegram Send Message's form, and its stored parameters (Bot API names). */

export interface InlineButton {
  text: string
  /** callback_data (sent back to the bot) or url (opened). */
  kind: 'callback' | 'url'
  value: string
}

export type Markup =
  | { kind: 'none' }
  | { kind: 'inline'; rows: InlineButton[][] }
  | { kind: 'keyboard'; rows: string[][]; resize: boolean; oneTime: boolean; placeholder: string }
  | { kind: 'remove' }
  | { kind: 'forceReply'; placeholder: string }

/** What the node sends: a text message, or a file by link (or file_id). */
export type TelegramOperation = 'sendMessage' | 'sendPhoto' | 'sendDocument' | 'sendVideo' | 'sendAudio' | 'sendAnimation'

export const TELEGRAM_OPERATIONS: { value: TelegramOperation; label: string; file?: string }[] = [
  { value: 'sendMessage', label: 'Send Message' },
  { value: 'sendPhoto', label: 'Send Photo', file: 'Photo' },
  { value: 'sendDocument', label: 'Send Document', file: 'Document' },
  { value: 'sendVideo', label: 'Send Video', file: 'Video' },
  { value: 'sendAudio', label: 'Send Audio', file: 'Audio' },
  { value: 'sendAnimation', label: 'Send Animation (GIF)', file: 'Animation' },
]

export interface MessageForm {
  operation: TelegramOperation
  /** The file's link, or a Telegram file_id (media operations). */
  file: string
  caption: string
  /** Media additional fields (each kind uses its own). */
  spoiler: boolean
  supportsStreaming: boolean
  duration: string
  performer: string
  title: string
  chatId: string
  text: string
  parseMode: '' | 'HTML' | 'Markdown' | 'MarkdownV2'
  markup: Markup
  disableNotification: boolean
  protectContent: boolean
  disableLinkPreview: boolean
  replyToMessageId: string
  messageThreadId: string
}

const str = (v: unknown) => (v === undefined || v === null ? '' : String(v))

function loadMarkup(m: unknown): Markup {
  const r = (m ?? {}) as Record<string, unknown>
  if (Array.isArray(r.inline_keyboard)) {
    return {
      kind: 'inline',
      rows: (r.inline_keyboard as Record<string, unknown>[][]).map((row) =>
        row.map((b) => (b.url !== undefined ? { text: str(b.text), kind: 'url', value: str(b.url) } : { text: str(b.text), kind: 'callback', value: str(b.callback_data) })),
      ),
    }
  }
  if (Array.isArray(r.keyboard)) {
    return {
      kind: 'keyboard',
      rows: (r.keyboard as Record<string, unknown>[][]).map((row) => row.map((b) => str(b.text))),
      resize: r.resize_keyboard === true,
      oneTime: r.one_time_keyboard === true,
      placeholder: str(r.input_field_placeholder),
    }
  }
  if (r.remove_keyboard === true) return { kind: 'remove' }
  if (r.force_reply === true) return { kind: 'forceReply', placeholder: str(r.input_field_placeholder) }
  return { kind: 'none' }
}

export function loadMessage(p: Record<string, unknown>): MessageForm {
  const mode = str(p.parse_mode)
  const op = TELEGRAM_OPERATIONS.find((o) => o.value === p.operation)?.value ?? 'sendMessage'
  return {
    operation: op,
    file: str(p.file),
    caption: str(p.caption),
    spoiler: p.has_spoiler === true,
    supportsStreaming: p.supports_streaming === true,
    duration: str(p.duration),
    performer: str(p.performer),
    title: str(p.title),
    // A new node answers the chat the incoming message came from.
    chatId: p.chat_id === undefined ? '{{ $json.message.chat.id }}' : str(p.chat_id),
    text: str(p.text),
    parseMode: (['HTML', 'Markdown', 'MarkdownV2'].includes(mode) ? mode : '') as MessageForm['parseMode'],
    markup: loadMarkup(p.reply_markup),
    disableNotification: p.disable_notification === true,
    protectContent: p.protect_content === true,
    disableLinkPreview: p.disable_web_page_preview === true,
    replyToMessageId: str(p.reply_to_message_id),
    messageThreadId: str(p.message_thread_id),
  }
}

/** A number when it is one ("7"), else the text (e.g. an expression). */
function numberOrText(v: string): number | string {
  return /^-?\d+$/.test(v.trim()) ? Number(v.trim()) : v.trim()
}

function buildMarkup(m: Markup): { markup?: Record<string, unknown>; error?: string } {
  switch (m.kind) {
    case 'inline': {
      const rows = m.rows.filter((r) => r.length > 0)
      for (const b of rows.flat()) {
        if (!b.text.trim()) return { error: 'Every button needs a label.' }
        if (!b.value.trim()) return { error: `Button "${b.text}" needs its URL or callback data.` }
      }
      return { markup: { inline_keyboard: rows.map((r) => r.map((b) => (b.kind === 'url' ? { text: b.text, url: b.value } : { text: b.text, callback_data: b.value }))) } }
    }
    case 'keyboard': {
      const rows = m.rows.map((r) => r.filter((t) => t.trim())).filter((r) => r.length > 0)
      if (rows.length === 0) return { error: 'Add at least one keyboard button.' }
      return {
        markup: {
          keyboard: rows.map((r) => r.map((text) => ({ text }))),
          ...(m.resize ? { resize_keyboard: true } : {}),
          ...(m.oneTime ? { one_time_keyboard: true } : {}),
          ...(m.placeholder.trim() ? { input_field_placeholder: m.placeholder } : {}),
        },
      }
    }
    case 'remove':
      return { markup: { remove_keyboard: true } }
    case 'forceReply':
      return { markup: { force_reply: true, ...(m.placeholder.trim() ? { input_field_placeholder: m.placeholder } : {}) } }
    default:
      return {}
  }
}

/** The node's parameters (Bot API names), or why the form can't be saved. */
export function buildMessage(f: MessageForm): { fields: Record<string, unknown> } | { error: string } {
  if (!f.chatId.trim()) return { error: 'Chat ID is required.' }
  const media = f.operation !== 'sendMessage'
  const what = TELEGRAM_OPERATIONS.find((o) => o.value === f.operation)?.file?.toLowerCase() ?? 'file'
  if (media && !f.file.trim()) return { error: `Enter the ${what}'s link (or a Telegram file_id).` }
  if (media && f.file.includes('://') && !/^https?:\/\//i.test(f.file.trim())) return { error: `The ${what}'s link must start with http:// or https://.` }
  if (!media && !f.text.trim()) return { error: 'Text is required.' }
  const markup = buildMarkup(f.markup)
  if (markup.error) return { error: markup.error }
  // The chat ID as typed: Telegram takes it as a number or a string.
  const fields: Record<string, unknown> = media
    ? { operation: f.operation, chat_id: f.chatId.trim(), file: f.file.trim(), ...(f.caption.trim() ? { caption: f.caption } : {}) }
    : { chat_id: f.chatId.trim(), text: f.text }
  if (f.parseMode) fields.parse_mode = f.parseMode
  if (markup.markup) fields.reply_markup = markup.markup
  if (f.disableNotification) fields.disable_notification = true
  if (f.protectContent) fields.protect_content = true
  if (f.disableLinkPreview && !media) fields.disable_web_page_preview = true
  const op = f.operation
  if (f.spoiler && ['sendPhoto', 'sendVideo', 'sendAnimation'].includes(op)) fields.has_spoiler = true
  if (f.supportsStreaming && op === 'sendVideo') fields.supports_streaming = true
  if (f.duration.trim() && ['sendVideo', 'sendAudio', 'sendAnimation'].includes(op)) {
    if (!/^\d+$/.test(f.duration.trim())) return { error: 'Duration is a whole number of seconds.' }
    fields.duration = Number(f.duration.trim())
  }
  if (op === 'sendAudio' && f.performer.trim()) fields.performer = f.performer.trim()
  if (op === 'sendAudio' && f.title.trim()) fields.title = f.title.trim()
  if (f.replyToMessageId.trim()) fields.reply_to_message_id = numberOrText(f.replyToMessageId)
  if (f.messageThreadId.trim()) fields.message_thread_id = numberOrText(f.messageThreadId)
  return { fields }
}
