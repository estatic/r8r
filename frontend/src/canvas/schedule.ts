/** The Schedule Trigger's and Wait node's forms, and their stored parameters. */

export type Interval = 'seconds' | 'minutes' | 'hours' | 'days' | 'weeks' | 'months' | 'cron'

export interface ScheduleForm {
  interval: Interval
  every: string
  hour: string
  minute: string
  weekdays: number[]
  dayOfMonth: string
  expression: string
  timezone: string
}

export const INTERVALS: { value: Interval; label: string }[] = [
  { value: 'seconds', label: 'Seconds' },
  { value: 'minutes', label: 'Minutes' },
  { value: 'hours', label: 'Hours' },
  { value: 'days', label: 'Days' },
  { value: 'weeks', label: 'Weeks' },
  { value: 'months', label: 'Months' },
  { value: 'cron', label: 'Custom (cron)' },
]
export const WEEKDAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
export const SCHEDULE_KEYS = ['rule', 'cron', 'timezone']

const str = (v: unknown, d = '') => (v === undefined || v === null ? d : String(v))

export function loadSchedule(p: Record<string, unknown>): ScheduleForm {
  const r = (p.rule ?? {}) as Record<string, unknown>
  const base: ScheduleForm = { interval: 'days', every: '1', hour: '9', minute: '0', weekdays: [1], dayOfMonth: '1', expression: '', timezone: str(p.timezone) }
  // Saved before rules: a 6-field cron (seconds first) shown as n8n's 5 fields when it can be.
  if (!p.rule && typeof p.cron === 'string') {
    const f = p.cron.trim().split(/\s+/)
    return { ...base, interval: 'cron', expression: f.length === 6 && f[0] === '0' ? f.slice(1).join(' ') : p.cron.trim() }
  }
  const interval = INTERVALS.some((i) => i.value === r.interval) ? (r.interval as Interval) : 'days'
  return {
    ...base,
    interval,
    every: str(r.every, '1'),
    hour: str(r.hour, '9'),
    minute: str(r.minute, '0'),
    weekdays: Array.isArray(r.weekdays) ? r.weekdays.filter((d): d is number => typeof d === 'number') : [1],
    dayOfMonth: str(r.day_of_month, '1'),
    expression: str(r.expression),
  }
}

function whole(v: string, name: string, min: number, max: number): number | string {
  const n = Number(v)
  if (v.trim() === '' || !Number.isInteger(n) || n < min || n > max) return `${name} must be a whole number from ${min} to ${max}.`
  return n
}

export function buildSchedule(f: ScheduleForm): { fields: Record<string, unknown> } | { error: string } {
  const rule: Record<string, unknown> = { interval: f.interval }
  const take = (key: string, v: string, name: string, min: number, max: number) => {
    const n = whole(v, name, min, max)
    if (typeof n === 'string') throw n
    rule[key] = n
  }
  try {
    switch (f.interval) {
      case 'seconds':
      case 'minutes':
        take('every', f.every, 'Every', 1, 59)
        break
      case 'hours':
        take('every', f.every, 'Every', 1, 23)
        take('minute', f.minute, 'Minute', 0, 59)
        break
      case 'days':
        take('every', f.every, 'Every', 1, 31)
        take('hour', f.hour, 'Hour', 0, 23)
        take('minute', f.minute, 'Minute', 0, 59)
        break
      case 'weeks':
        if (f.weekdays.length === 0) throw 'Choose at least one day of the week.'
        rule.weekdays = [...f.weekdays].sort()
        take('hour', f.hour, 'Hour', 0, 23)
        take('minute', f.minute, 'Minute', 0, 59)
        break
      case 'months':
        take('every', f.every, 'Every', 1, 12)
        take('day_of_month', f.dayOfMonth, 'Day of month', 1, 31)
        take('hour', f.hour, 'Hour', 0, 23)
        take('minute', f.minute, 'Minute', 0, 59)
        break
      case 'cron': {
        const n = f.expression.trim().split(/\s+/).filter(Boolean).length
        if (n !== 5 && n !== 6) throw 'A cron expression has 5 fields: minute hour day month weekday.'
        rule.expression = f.expression.trim()
        break
      }
    }
  } catch (e) {
    return { error: String(e) }
  }
  const fields: Record<string, unknown> = { rule }
  if (f.timezone.trim()) fields.timezone = f.timezone.trim()
  return { fields }
}

/** In words, for the form: "Every 15 minutes", "Mondays and Fridays at 08:00". */
export function describeSchedule(f: ScheduleForm): string {
  const at = `${f.hour.padStart(2, '0')}:${f.minute.padStart(2, '0')}`
  const every = (unit: string) => (f.every === '1' ? `Every ${unit}` : `Every ${f.every} ${unit}s`)
  switch (f.interval) {
    case 'seconds':
      return every('second')
    case 'minutes':
      return every('minute')
    case 'hours':
      return `${every('hour')} at minute ${f.minute}`
    case 'days':
      return `${every('day')} at ${at}`
    case 'weeks':
      return `${f.weekdays.map((d) => WEEKDAY_NAMES[d]).join(', ') || '—'} at ${at}`
    case 'months':
      return `${every('month')} on day ${f.dayOfMonth} at ${at}`
    default:
      return `Cron: ${f.expression || '—'}`
  }
}

export interface WaitForm {
  resume: 'interval' | 'at'
  amount: string
  unit: 'seconds' | 'minutes' | 'hours' | 'days'
  dateTime: string
  timezone: string
}

export const WAIT_KEYS = ['resume', 'amount', 'unit', 'date_time', 'timezone', 'seconds']

export function loadWait(p: Record<string, unknown>): WaitForm {
  const base: WaitForm = { resume: 'interval', amount: '5', unit: 'seconds', dateTime: '', timezone: str(p.timezone) }
  if (p.resume === undefined && p.seconds !== undefined) return { ...base, amount: str(p.seconds) }
  const unit = ['seconds', 'minutes', 'hours', 'days'].includes(p.unit as string) ? (p.unit as WaitForm['unit']) : 'seconds'
  return { ...base, resume: p.resume === 'at' ? 'at' : 'interval', amount: str(p.amount, '5'), unit, dateTime: str(p.date_time) }
}

export function buildWait(f: WaitForm): { fields: Record<string, unknown> } | { error: string } {
  if (f.resume === 'at') {
    if (!f.dateTime.trim()) return { error: 'Enter the date and time to wait until.' }
    const fields: Record<string, unknown> = { resume: 'at', date_time: f.dateTime.trim() }
    if (f.timezone.trim()) fields.timezone = f.timezone.trim()
    return { fields }
  }
  const raw = f.amount.trim()
  // A number, or an expression that gives one.
  const amount = /^\d+(\.\d+)?$/.test(raw) ? Number(raw) : raw
  if (amount === '') return { error: 'Enter how long to wait.' }
  return { fields: { resume: 'interval', amount, unit: f.unit } }
}
