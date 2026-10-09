import { describe, expect, it } from 'vitest'
import { buildSchedule, buildWait, describeSchedule, loadSchedule, loadWait } from './schedule'

describe('Schedule form', () => {
  it('starts as every day at 09:00', () => {
    const f = loadSchedule({})
    expect(describeSchedule(f)).toBe('Every day at 09:00')
    expect(buildSchedule(f)).toEqual({ fields: { rule: { interval: 'days', every: 1, hour: 9, minute: 0 } } })
  })

  it('round-trips each interval with a time zone', () => {
    for (const rule of [
      { interval: 'minutes', every: 15 },
      { interval: 'hours', every: 2, minute: 30 },
      { interval: 'weeks', weekdays: [1, 5], hour: 8, minute: 0 },
      { interval: 'months', every: 1, day_of_month: 15, hour: 12, minute: 0 },
      { interval: 'cron', expression: '*/5 9-17 * * 1-5' },
    ]) {
      expect(buildSchedule(loadSchedule({ rule, timezone: 'Europe/Berlin' }))).toEqual({ fields: { rule, timezone: 'Europe/Berlin' } })
    }
  })

  it('shows an older 6-field cron as the 5 fields n8n uses', () => {
    expect(loadSchedule({ cron: '0 30 9 * * Mon' })).toMatchObject({ interval: 'cron', expression: '30 9 * * Mon' })
  })

  it('says what is wrong', () => {
    expect(buildSchedule({ ...loadSchedule({}), hour: '25' })).toEqual({ error: 'Hour must be a whole number from 0 to 23.' })
    expect(buildSchedule({ ...loadSchedule({ rule: { interval: 'weeks' } }), weekdays: [] })).toEqual({ error: 'Choose at least one day of the week.' })
    expect(buildSchedule({ ...loadSchedule({ rule: { interval: 'cron' } }), expression: 'daily' })).toHaveProperty('error')
  })
})

describe('Wait form', () => {
  it('round-trips an interval and a time', () => {
    expect(buildWait(loadWait({ resume: 'interval', amount: 2, unit: 'hours' }))).toEqual({ fields: { resume: 'interval', amount: 2, unit: 'hours' } })
    const at = { resume: 'at', date_time: '2026-10-09T18:30', timezone: 'Europe/Berlin' }
    expect(buildWait(loadWait(at))).toEqual({ fields: at })
  })

  it('loads an older seconds value', () => {
    expect(buildWait(loadWait({ seconds: 3 }))).toEqual({ fields: { resume: 'interval', amount: 3, unit: 'seconds' } })
  })

  it('needs a time to wait until', () => {
    expect(buildWait({ ...loadWait({}), resume: 'at' })).toEqual({ error: 'Enter the date and time to wait until.' })
  })
})
