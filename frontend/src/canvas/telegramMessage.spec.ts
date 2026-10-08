import { describe, it, expect } from 'vitest'
import { buildMessage, loadMessage } from './telegramMessage'

describe('Telegram message form', () => {
  it('starts a new node replying to the chat the message came from', () => {
    const m = loadMessage({})
    expect(m.chatId).toBe('{{ $json.message.chat.id }}')
    expect(m.markup.kind).toBe('none')
  })

  it('round-trips text, parse mode and the additional fields', () => {
    const params = {
      chat_id: '{{ $json.message.chat.id }}', text: 'Hi *there*', parse_mode: 'MarkdownV2',
      disable_notification: true, protect_content: true, disable_web_page_preview: true,
      reply_to_message_id: '{{ $json.message.message_id }}', message_thread_id: 7,
    }
    expect(buildMessage(loadMessage(params))).toEqual({ fields: params })
  })

  it('round-trips an inline keyboard of callback and URL buttons', () => {
    const reply_markup = { inline_keyboard: [[{ text: 'Yes', callback_data: 'yes' }, { text: 'Site', url: 'https://example.com' }], [{ text: 'No', callback_data: 'no' }]] }
    const m = loadMessage({ chat_id: '1', text: 't', reply_markup })
    expect(m.markup.kind).toBe('inline')
    expect(buildMessage(m)).toEqual({ fields: { chat_id: '1', text: 't', reply_markup } })
  })

  it('round-trips a reply keyboard with its options, and the simple markups', () => {
    const keyboard = { keyboard: [[{ text: 'A' }, { text: 'B' }]], resize_keyboard: true, one_time_keyboard: true, input_field_placeholder: 'Pick one' }
    expect(buildMessage(loadMessage({ chat_id: '1', text: 't', reply_markup: keyboard }))).toEqual({ fields: { chat_id: '1', text: 't', reply_markup: keyboard } })
    for (const markup of [{ remove_keyboard: true }, { force_reply: true, input_field_placeholder: 'Answer' }]) {
      expect(buildMessage(loadMessage({ chat_id: '1', text: 't', reply_markup: markup }))).toEqual({ fields: { chat_id: '1', text: 't', reply_markup: markup } })
    }
  })

  it('leaves unset options out', () => {
    const m = loadMessage({ chat_id: '1', text: 't' })
    expect(buildMessage(m)).toEqual({ fields: { chat_id: '1', text: 't' } })
  })

  it('needs a chat, a text, and complete buttons', () => {
    expect(buildMessage({ ...loadMessage({ text: 't' }), chatId: ' ' })).toEqual({ error: 'Chat ID is required.' })
    expect(buildMessage(loadMessage({ chat_id: '1', text: '' }))).toEqual({ error: 'Text is required.' })
    const m = loadMessage({ chat_id: '1', text: 't', reply_markup: { inline_keyboard: [[{ text: '', callback_data: 'x' }]] } })
    expect(buildMessage(m)).toEqual({ error: 'Every button needs a label.' })
    const n = loadMessage({ chat_id: '1', text: 't', reply_markup: { inline_keyboard: [[{ text: 'Go', url: '' }]] } })
    expect(buildMessage(n)).toEqual({ error: 'Button "Go" needs its URL or callback data.' })
  })
})
