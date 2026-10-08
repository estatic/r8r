export type CodeLanguage = 'javaScript' | 'python'

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
