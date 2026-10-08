import { describe, it, expect } from 'vitest'
import { ApiError } from './client'
import { errorText } from './errorText'

describe('errorText', () => {
  it("shows the server's own explanation", () => {
    const e = new ApiError(409, 'This workflow is active, so the bot\'s messages run it by themselves.')
    expect(errorText(e, 'Failed to execute workflow.')).toBe("Failed to execute workflow. This workflow is active, so the bot's messages run it by themselves.")
  })
  it('falls back to the status when the server said nothing useful', () => {
    expect(errorText(new ApiError(500, 'request failed with status 500'), 'Failed to save.')).toBe('Failed to save. (500)')
    expect(errorText(new ApiError(502, ''), 'Failed to save.')).toBe('Failed to save. (502)')
  })
  it('does not dump an HTML error page', () => {
    expect(errorText(new ApiError(502, '<html><body>Bad Gateway</body></html>'), 'Failed.')).toBe('Failed. (502)')
  })
  it('uses the fallback for anything else', () => {
    expect(errorText(new Error('boom'), 'Failed.')).toBe('Failed.')
  })
})
