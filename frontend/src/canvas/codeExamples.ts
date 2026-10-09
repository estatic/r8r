export type CodeLanguage = 'javaScript' | 'python'
/** n8n's names: run the code once with all items, or once per item. */
export type CodeMode = 'runOnceForAllItems' | 'runOnceForEachItem'

/** What a new Code node starts with: a small, working example per language. */
export const CODE_EXAMPLES: Record<CodeLanguage, string> = {
  javaScript: `// items: the input items, each with item.json
// Return the items to pass on.
return items.map((item) => ({
  json: { ...item.json, processed: true },
}))
`,
  python: `# items: the input items, each with item.json
# Return the items to pass on.
return [
    {"json": {**item.json, "processed": True}}
    for item in items
]
`,
}

/** The same, run once for each item: it returns that one item. */
export const EACH_ITEM_EXAMPLES: Record<CodeLanguage, string> = {
  javaScript: `// $json: this item's data. Runs once for each input item.
// Return this item (one object).
return { ...$json, processed: true }
`,
  python: `# _json: this item's data. Runs once for each input item.
# Return this item (one dict).
return {**_json, "processed": True}
`,
}

export function codeExample(language: CodeLanguage, mode: CodeMode): string {
  return (mode === 'runOnceForEachItem' ? EACH_ITEM_EXAMPLES : CODE_EXAMPLES)[language]
}

/** An empty box or one of the examples: safe to swap for another example. */
export function isCodeExample(script: string): boolean {
  return script.trim() === '' || [...Object.values(CODE_EXAMPLES), ...Object.values(EACH_ITEM_EXAMPLES)].some((ex) => ex.trim() === script.trim())
}
