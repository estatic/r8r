# Node Property Schemas (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** every core node of the n8n-compatible engine is described by a real parameter schema at `/types/nodes.json`, and the editor has a generic component that renders and edits any node's parameters from that schema.

**Architecture:** a small Rust builder (`src/n8n/schema/`) produces n8n-shaped property JSON (`displayName`, `name`, `type`, `default`, `options`, `displayOptions`, `typeOptions`), one file per node family; `node_types::describe` uses a node's schema when it has one. On the frontend, `src/schema/` evaluates `displayOptions` and `NodeParameters.vue` renders the visible properties recursively, emitting a new parameters object. Schemas are written clean-room from r8r's own engine code (what each node reads), never copied from n8n (spec §9).

**Tech Stack:** Rust (serde_json), cucumber BDD (`tests/bdd`), Vue 3 + TypeScript, Vitest.

**Spec:** `raw/2026-09-25-n8n-in-rust-reimplementation-spec.md` §4.3, §6.6, §9; roadmap `docs/superpowers/plans/2026-10-08-editor-on-n8n-engine-roadmap.md`.

## Global Constraints

- Shape: n8n's `INodeTypeDescription` / `INodeProperties` field names and type names (spec §4.3: "Same `INodeTypeDescription` JSON served at `/types/nodes.json`").
- Clean-room: no n8n source, descriptions, or assets copied (spec §9). Schemas come from what `src/n8n/nodes/*.rs` reads; wording is r8r's own.
- A schema must list every parameter path its node reads, with the engine's default.
- `07-api` BDD scenarios must stay green on r8r **and** n8n 2.35.7 (`tests/bdd/README.md`, "Checking the expectations against n8n").
- Each task ≤ 20 minutes; end each with the `=== CHECKPOINT ===` block.

## Review Focus

1. **A parameter the engine reads but the schema omits:** the form can't set it, so the node silently uses its default. Task 1's coverage test fails for any `params` entry missing from a schema.
2. **`displayOptions` naming a property that doesn't exist:** the field never shows. Task 1's test checks every `show`/`hide` key names a property of the same node (or `@version`).
3. **An `options` default that isn't one of its options:** the form shows nothing selected. Task 1's test checks it.
4. **Hidden-but-set values:** a value set, then hidden by another choice, must be dropped from the saved parameters, as n8n does. Task 4's test covers it.
5. **Expressions in any field:** `={{ … }}` must survive editing of every property type, including numbers and booleans. Task 4's test covers it.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `src/n8n/schema/mod.rs` | Builder (`Prop`, `string`, `number`, `boolean`, `options`, `multi_options`, `json`, `notice`, `collection`, `fixed_collection`, `assignments`, `filter`), `for_node(name) -> Option<Vec<Value>>` |
| `src/n8n/schema/base.rs` | Triggers and flow: manualTrigger, scheduleTrigger, webhook, noOp, wait, limit, splitInBatches, splitOut, merge, code |
| `src/n8n/schema/fields.rs` | set (assignments), if, filter, switch (conditions) |
| `src/n8n/schema/http.rs` | httpRequest |
| `src/n8n/schema/telegram.rs` | telegram (message resource) |
| `src/n8n/node_types.rs` | `describe` uses `schema::for_node`; tests |
| `frontend/src/schema/types.ts` | `NodeProperty`, `NodeTypeDescription` TS types |
| `frontend/src/schema/visibility.ts` | `isVisible(prop, params, version)`, `pruneHidden(props, params, version)` |
| `frontend/src/components/params/NodeParameters.vue` | Renders a property list; `v-model` = parameters object |
| `frontend/src/components/params/ParameterInput.vue` | One property (switches on `type`) |
| `frontend/src/components/params/AssignmentsInput.vue` | `assignmentCollection` (Set) |
| `frontend/src/components/params/FilterInput.vue` | `filter` (If/Filter conditions) |

---

### Task 1: Schema builder, schema-aware `describe`, and consistency tests

**Files:**
- Create: `src/n8n/schema/mod.rs`, `src/n8n/schema/base.rs`
- Modify: `src/n8n/mod.rs` (add `pub mod schema;`), `src/n8n/node_types.rs:416-434`

**Interfaces:**
- Produces: `pub struct Prop(pub serde_json::Value)` with `.desc(&str)`, `.placeholder(&str)`, `.required()`, `.rows(u32)`, `.password()`, `.code(lang: &str)`, `.min(f64)`, `.show(param: &str, values: Value)`, `.hide(param: &str, values: Value)`; constructors below; `pub fn for_node(name: &str) -> Option<Vec<serde_json::Value>>`.

- [ ] **Step 1: Write the failing tests** — append to `src/n8n/node_types.rs` tests module:

```rust
    #[test]
    fn a_schema_covers_every_parameter_its_node_reads() {
        for d in DESCRIPTIONS {
            let Some(props) = crate::n8n::schema::for_node(d.name) else { continue };
            let names: Vec<&str> = props.iter().filter_map(|p| p["name"].as_str()).collect();
            for param in d.params {
                assert!(names.contains(param), "{}: schema lacks \"{param}\"", d.name);
            }
        }
    }

    #[test]
    fn display_options_name_real_properties_and_defaults_are_options() {
        for d in DESCRIPTIONS {
            let Some(props) = crate::n8n::schema::for_node(d.name) else { continue };
            let names: Vec<&str> = props.iter().filter_map(|p| p["name"].as_str()).collect();
            for p in &props {
                for rule in ["show", "hide"] {
                    if let Some(map) = p.pointer(&format!("/displayOptions/{rule}")).and_then(|v| v.as_object()) {
                        for key in map.keys() {
                            assert!(key == "@version" || names.contains(&key.as_str()), "{}: {} {rule}s on unknown \"{key}\"", d.name, p["name"]);
                        }
                    }
                }
                if p["type"] == "options" {
                    let values: Vec<&Value> = p["options"].as_array().unwrap().iter().map(|o| &o["value"]).collect();
                    assert!(values.contains(&&p["default"]), "{}: {} default isn't an option", d.name, p["name"]);
                }
            }
        }
    }

    #[test]
    fn a_node_with_a_schema_is_served_with_it() {
        let limit = describe(DESCRIPTIONS.iter().find(|d| d.name == "n8n-nodes-base.limit").unwrap());
        let max = limit["properties"].as_array().unwrap().iter().find(|p| p["name"] == "maxItems").unwrap();
        assert_eq!(max["type"], "number");
        assert_eq!(max["default"], 1);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib node_types:: 2>&1 | grep -E "error|FAILED|test result"`
Expected: compile error, `crate::n8n::schema` doesn't exist.

- [ ] **Step 3: Write the builder** — `src/n8n/schema/mod.rs`:

```rust
//! Clean-room parameter schemas in n8n's `INodeProperties` shape (spec §4.3),
//! written from what r8r's own nodes read (never copied from n8n, spec §9).

use serde_json::{json, Value};

mod base;

/// One property; builder methods add the optional fields.
pub struct Prop(pub Value);

impl Prop {
    fn new(name: &str, display: &str, kind: &str, default: Value) -> Self {
        Prop(json!({"displayName": display, "name": name, "type": kind, "default": default}))
    }
    pub fn desc(mut self, text: &str) -> Self {
        self.0["description"] = json!(text);
        self
    }
    pub fn placeholder(mut self, text: &str) -> Self {
        self.0["placeholder"] = json!(text);
        self
    }
    pub fn required(mut self) -> Self {
        self.0["required"] = json!(true);
        self
    }
    pub fn rows(mut self, rows: u32) -> Self {
        self.0["typeOptions"]["rows"] = json!(rows);
        self
    }
    pub fn password(mut self) -> Self {
        self.0["typeOptions"]["password"] = json!(true);
        self
    }
    /// A code editor for `lang` ("javaScript", "python", "json", "html", "sql").
    pub fn code(mut self, lang: &str) -> Self {
        self.0["typeOptions"]["editor"] = json!("codeNodeEditor");
        self.0["typeOptions"]["editorLanguage"] = json!(lang);
        self
    }
    pub fn min(mut self, value: f64) -> Self {
        self.0["typeOptions"]["minValue"] = json!(value);
        self
    }
    /// Shown only while `param` has one of `values` (an array).
    pub fn show(mut self, param: &str, values: Value) -> Self {
        self.0["displayOptions"]["show"][param] = values;
        self
    }
    pub fn hide(mut self, param: &str, values: Value) -> Self {
        self.0["displayOptions"]["hide"][param] = values;
        self
    }
}

pub fn string(name: &str, display: &str, default: &str) -> Prop {
    Prop::new(name, display, "string", json!(default))
}
pub fn number(name: &str, display: &str, default: f64) -> Prop {
    let d = if default.fract() == 0.0 { json!(default as i64) } else { json!(default) };
    Prop::new(name, display, "number", d)
}
pub fn boolean(name: &str, display: &str, default: bool) -> Prop {
    Prop::new(name, display, "boolean", json!(default))
}
pub fn json_input(name: &str, display: &str, default: &str) -> Prop {
    Prop::new(name, display, "json", json!(default))
}
pub fn notice(name: &str, text: &str) -> Prop {
    Prop::new(name, text, "notice", json!(""))
}
/// A single choice from `(label, value)` pairs.
pub fn options(name: &str, display: &str, default: &str, choices: &[(&str, &str)]) -> Prop {
    let mut p = Prop::new(name, display, "options", json!(default));
    p.0["options"] = json!(choices.iter().map(|(label, value)| json!({"name": label, "value": value})).collect::<Vec<_>>());
    p
}
pub fn multi_options(name: &str, display: &str, choices: &[(&str, &str)]) -> Prop {
    let mut p = Prop::new(name, display, "multiOptions", json!([]));
    p.0["options"] = json!(choices.iter().map(|(label, value)| json!({"name": label, "value": value})).collect::<Vec<_>>());
    p
}
/// Optional fields under one name ("Options", "Additional Fields").
pub fn collection(name: &str, display: &str, placeholder: &str, fields: Vec<Prop>) -> Prop {
    let mut p = Prop::new(name, display, "collection", json!({}));
    p.0["placeholder"] = json!(placeholder);
    p.0["options"] = json!(fields.into_iter().map(|f| f.0).collect::<Vec<_>>());
    p
}
/// Named groups of fields; `multiple` lets each group repeat (rows).
pub fn fixed_collection(name: &str, display: &str, multiple: bool, groups: Vec<(&str, &str, Vec<Prop>)>) -> Prop {
    let mut p = Prop::new(name, display, "fixedCollection", json!({}));
    if multiple {
        p.0["typeOptions"]["multipleValues"] = json!(true);
    }
    p.0["options"] = json!(groups
        .into_iter()
        .map(|(name, display, values)| json!({"name": name, "displayName": display, "values": values.into_iter().map(|v| v.0).collect::<Vec<_>>()}))
        .collect::<Vec<_>>());
    p
}

/// The schema of `name`, if r8r has one.
pub fn for_node(name: &str) -> Option<Vec<Value>> {
    base::for_node(name).map(|props| props.into_iter().map(|p| p.0).collect())
}
```

`src/n8n/schema/base.rs` (Limit and Code fully; the other core nodes in Task 2):

```rust
use super::*;

pub(super) fn for_node(name: &str) -> Option<Vec<Prop>> {
    Some(match name.trim_start_matches("n8n-nodes-base.") {
        "limit" => vec![
            number("maxItems", "Max Items", 1.0).min(1.0).desc("How many items to keep"),
            options("keep", "Keep", "firstItems", &[("First Items", "firstItems"), ("Last Items", "lastItems")]),
        ],
        "code" => vec![
            options("mode", "Mode", "runOnceForAllItems", &[("Run Once for All Items", "runOnceForAllItems"), ("Run Once for Each Item", "runOnceForEachItem")]),
            options("language", "Language", "javaScript", &[("JavaScript", "javaScript"), ("Python", "python")]),
            string("jsCode", "JavaScript", "return $input.all();").code("javaScript").show("language", json!(["javaScript"])),
            string("pythonCode", "Python", "return _items").code("python").show("language", json!(["python"])),
        ],
        _ => return None,
    })
}
```

- [ ] **Step 4: Use the schema in `describe`** — `src/n8n/node_types.rs:416`, first line of `describe`:

```rust
    let properties: Vec<Value> = super::schema::for_node(desc.name).unwrap_or_else(|| {
        desc.params.iter().map(|p| json!({"displayName": p, "name": p, "type": "string", "default": ""})).collect()
    });
```

and add `pub mod schema;` to `src/n8n/mod.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib node_types:: 2>&1 | grep -E "FAILED|test result"`
Expected: PASS (all node_types tests).

- [ ] **Step 6: Check the 07-api scenarios on r8r**

Run: `cargo test --test bdd -- -i 'tests/bdd/features/07-api/rest_auth_and_editor.feature' 2>&1 | grep -E "scenarios \("`
Expected: all passed.

- [ ] **Step 7: Commit**

```bash
git add src/n8n/schema src/n8n/mod.rs src/n8n/node_types.rs
git commit -m "feat(n8n): node property schemas (builder, Limit, Code) served at /types/nodes.json"
```

### Task 2: Schemas for the remaining core nodes

**Files:**
- Modify: `src/n8n/schema/base.rs`

**Interfaces:**
- Consumes: the Task 1 builder.

For each node below, list the paths its engine code reads, then write its schema so each path appears with the engine's default. The coverage test from Task 1 fails until every `params` entry of `DESCRIPTIONS` has a property.

- [ ] **Step 1: List what each node reads**

Run (one per node file; read the matching struct's `execute`):
```bash
grep -nE '(param_[a-z0-9_]+|raw_param)\("[a-zA-Z0-9_.]+"' src/n8n/nodes/{core,transform,merge,utility,server_nodes}.rs
```
Expected: lines like `param_f64("batchSize", …)`, `param_bool("options.reset", …)`, `raw_param("mergeByFields.values")`.

- [ ] **Step 2: Write schemas** for `manualTrigger`, `scheduleTrigger`, `webhook`, `noOp`, `wait`, `splitInBatches`, `splitOut`, `merge`, `aggregate`, `summarize`, `sort`, `removeDuplicates`. Rules:
  - a dotted path `options.x` → a `collection("options", "Options", "Add option", vec![...x...])`;
  - `a.b` read as a list (`raw_param("fieldsToAggregate.fieldToAggregate")`) → `fixed_collection("fieldsToAggregate", …, true, vec![("fieldToAggregate", …, vec![...])])`;
  - a string compared against fixed values in the engine (`match … { "firstItems" => …, "lastItems" => … }`) → `options` with exactly those values and the engine's default;
  - a field used only for some choice of another (`if resume == "timeInterval"`) → `.show("resume", json!(["timeInterval"]))`.

- [ ] **Step 3: Run the consistency tests**

Run: `cargo test --lib node_types:: 2>&1 | grep -E "FAILED|panicked|test result"`
Expected: PASS. A failure names the node and the missing or inconsistent property.

- [ ] **Step 4: Commit** `feat(n8n): schemas for triggers and flow nodes`.

### Task 3: Frontend schema types and visibility

**Files:**
- Create: `frontend/src/schema/types.ts`, `frontend/src/schema/visibility.ts`, `frontend/src/schema/visibility.spec.ts`

**Interfaces:**
- Produces: `NodeProperty`, `NodeTypeDescription`; `isVisible(prop: NodeProperty, params: Record<string, unknown>, version: number): boolean`; `pruneHidden(props: NodeProperty[], params: Record<string, unknown>, version: number): Record<string, unknown>`.

- [ ] **Step 1: Write the failing tests** — `frontend/src/schema/visibility.spec.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { isVisible, pruneHidden } from './visibility'
import type { NodeProperty } from './types'

const js: NodeProperty = { displayName: 'JavaScript', name: 'jsCode', type: 'string', default: '', displayOptions: { show: { language: ['javaScript'] } } }
const py: NodeProperty = { displayName: 'Python', name: 'pythonCode', type: 'string', default: '', displayOptions: { hide: { language: ['javaScript'] } } }
const v2: NodeProperty = { displayName: 'New', name: 'n', type: 'string', default: '', displayOptions: { show: { '@version': [2] } } }

describe('isVisible', () => {
  it('shows a property while its condition holds, using the default when unset', () => {
    expect(isVisible(js, { language: 'javaScript' }, 1)).toBe(true)
    expect(isVisible(js, { language: 'python' }, 1)).toBe(false)
    expect(isVisible(py, { language: 'python' }, 1)).toBe(true)
  })
  it('understands @version', () => {
    expect(isVisible(v2, {}, 2)).toBe(true)
    expect(isVisible(v2, {}, 1)).toBe(false)
  })
})

describe('pruneHidden', () => {
  it('drops values whose property is hidden, keeps expressions', () => {
    const lang: NodeProperty = { displayName: 'Language', name: 'language', type: 'options', default: 'javaScript', options: [{ name: 'JS', value: 'javaScript' }, { name: 'Py', value: 'python' }] }
    expect(pruneHidden([lang, js, py], { language: 'python', jsCode: 'x', pythonCode: '={{ $json.code }}' }, 1)).toEqual({ language: 'python', pythonCode: '={{ $json.code }}' })
  })
})
```

- [ ] **Step 2: Run to see it fail**

Run: `cd frontend && npx vitest run src/schema 2>&1 | grep -E "Error|Tests"`
Expected: FAIL, `./visibility` not found.

- [ ] **Step 3: Implement** — `frontend/src/schema/types.ts`:

```ts
export interface PropertyOption {
  name: string
  value: string | number | boolean
  description?: string
}

export interface NodeProperty {
  displayName: string
  name: string
  type: 'string' | 'number' | 'boolean' | 'options' | 'multiOptions' | 'json' | 'notice' | 'collection' | 'fixedCollection' | 'assignmentCollection' | 'filter' | 'dateTime' | 'color' | 'hidden'
  default: unknown
  description?: string
  placeholder?: string
  required?: boolean
  /** options: choices; collection: fields; fixedCollection: groups ({ name, displayName, values }). */
  options?: (PropertyOption | NodeProperty | { name: string; displayName: string; values: NodeProperty[] })[]
  typeOptions?: { rows?: number; password?: boolean; editor?: string; editorLanguage?: string; minValue?: number; maxValue?: number; multipleValues?: boolean }
  displayOptions?: { show?: Record<string, unknown[]>; hide?: Record<string, unknown[]> }
}

export interface NodeTypeDescription {
  name: string
  displayName: string
  version: number | number[]
  description: string
  group: string[]
  inputs: unknown[]
  outputs: unknown[]
  properties: NodeProperty[]
  credentials?: { name: string; required?: boolean }[]
}
```

`frontend/src/schema/visibility.ts`:

```ts
import type { NodeProperty } from './types'

function current(name: string, params: Record<string, unknown>, all: NodeProperty[] | undefined) {
  if (name in params) return params[name]
  return all?.find((p) => p.name === name)?.default
}

/** n8n's displayOptions: every `show` key must match, no `hide` key may. */
export function isVisible(prop: NodeProperty, params: Record<string, unknown>, version: number, all?: NodeProperty[]): boolean {
  const show = prop.displayOptions?.show ?? {}
  const hide = prop.displayOptions?.hide ?? {}
  const value = (key: string) => (key === '@version' ? version : current(key, params, all))
  for (const [key, allowed] of Object.entries(show)) if (!allowed.includes(value(key))) return false
  for (const [key, denied] of Object.entries(hide)) if (denied.includes(value(key))) return false
  return true
}

/** `params` without the values of properties that are now hidden. */
export function pruneHidden(props: NodeProperty[], params: Record<string, unknown>, version: number): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const p of props) {
    if (p.name in params && isVisible(p, params, version, props)) out[p.name] = params[p.name]
  }
  return out
}
```

- [ ] **Step 4: Run tests** — `cd frontend && npx vitest run src/schema`; Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): node schema types and displayOptions`.

### Task 4: Generic parameter form

**Files:**
- Create: `frontend/src/components/params/NodeParameters.vue`, `frontend/src/components/params/ParameterInput.vue`, `frontend/src/components/params/NodeParameters.spec.ts`

**Interfaces:**
- Consumes: `isVisible`, `pruneHidden`, `NodeProperty` (Task 3); `PromptBox.vue` (multi-line and code fields).
- Produces: `<NodeParameters :properties :version v-model="parameters" />` emitting a parameters object without hidden values.

- [ ] **Step 1: Write the failing tests** — `NodeParameters.spec.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import NodeParameters from './NodeParameters.vue'
import type { NodeProperty } from '../../schema/types'

const props: NodeProperty[] = [
  { displayName: 'Max Items', name: 'maxItems', type: 'number', default: 1 },
  { displayName: 'Keep', name: 'keep', type: 'options', default: 'firstItems', options: [{ name: 'First', value: 'firstItems' }, { name: 'Last', value: 'lastItems' }] },
  { displayName: 'Language', name: 'language', type: 'options', default: 'javaScript', options: [{ name: 'JS', value: 'javaScript' }, { name: 'Py', value: 'python' }] },
  { displayName: 'JavaScript', name: 'jsCode', type: 'string', default: '', typeOptions: { editor: 'codeNodeEditor' }, displayOptions: { show: { language: ['javaScript'] } } },
  { displayName: 'Options', name: 'options', type: 'collection', default: {}, placeholder: 'Add option', options: [{ displayName: 'Reset', name: 'reset', type: 'boolean', default: false }] },
]
const last = (w: ReturnType<typeof mount>) => (w.emitted('update:modelValue')!.at(-1)![0] as Record<string, unknown>)

describe('NodeParameters', () => {
  it('shows each visible property with its default', () => {
    const w = mount(NodeParameters, { props: { properties: props, version: 1, modelValue: {} } })
    expect((w.find('input[aria-label="Max Items"]').element as HTMLInputElement).value).toBe('1')
    expect(w.find('[aria-label="JavaScript"]').exists()).toBe(true)
  })

  it('edits a number as a number and keeps an expression', async () => {
    const w = mount(NodeParameters, { props: { properties: props, version: 1, modelValue: {} } })
    await w.find('input[aria-label="Max Items"]').setValue('5')
    expect(last(w).maxItems).toBe(5)
    await w.find('input[aria-label="Max Items"]').setValue('={{ $json.n }}')
    expect(last(w).maxItems).toBe('={{ $json.n }}')
  })

  it('drops a value when its property becomes hidden', async () => {
    const w = mount(NodeParameters, { props: { properties: props, version: 1, modelValue: { language: 'javaScript', jsCode: 'x' } } })
    await w.find('select[aria-label="Language"]').setValue('python')
    expect(last(w)).toEqual({ language: 'python' })
  })

  it('adds an optional field from a collection', async () => {
    const w = mount(NodeParameters, { props: { properties: props, version: 1, modelValue: {} } })
    await w.find('select[aria-label="Add option"]').setValue('reset')
    expect(last(w).options).toEqual({ reset: false })
  })
})
```

- [ ] **Step 2: Run to see it fail** — `cd frontend && npx vitest run src/components/params`; Expected: FAIL, component missing.

- [ ] **Step 3: Implement** `ParameterInput.vue`: one property, chosen by `type`: `string` → `<input>` (or `PromptBox` when `typeOptions.rows` or `editor`; `password` → `type="password"`), `number` → numeric `<input type="text">` that emits a number for numeric text and the raw string when it starts with `=`, `boolean` → checkbox, `options` → `<select>`, `multiOptions` → checkboxes, `json` → `PromptBox code`, `notice` → a `<p>`, `collection` → its set fields (each a nested `ParameterInput`, with ✕) plus an `Add option` `<select aria-label="{placeholder}">` of the unset ones, `fixedCollection` → per group: rows of nested `NodeParameters` (multiple) or one, with + / ✕. Every control has `aria-label="{displayName}"`.

`NodeParameters.vue`:

```vue
<script setup lang="ts">
import { computed } from 'vue'
import ParameterInput from './ParameterInput.vue'
import { isVisible, pruneHidden } from '../../schema/visibility'
import type { NodeProperty } from '../../schema/types'

/** A form for a node's parameters, drawn from its schema. */
const params = defineModel<Record<string, unknown>>({ required: true })
const props = defineProps<{ properties: NodeProperty[]; version: number }>()

const visible = computed(() => props.properties.filter((p) => isVisible(p, params.value, props.version, props.properties)))

function set(name: string, value: unknown) {
  params.value = pruneHidden(props.properties, { ...params.value, [name]: value }, props.version)
}
</script>

<template>
  <div class="space-y-2">
    <ParameterInput
      v-for="p in visible"
      :key="p.name"
      :property="p"
      :version="version"
      :model-value="p.name in params ? params[p.name] : p.default"
      @update:model-value="(v: unknown) => set(p.name, v)"
    />
  </div>
</template>
```

- [ ] **Step 4: Run tests** — `cd frontend && npx vitest run src/components/params && npx vue-tsc --noEmit -p tsconfig.json`; Expected: PASS, no type errors.
- [ ] **Step 5: Commit** `feat(editor): generic schema-driven parameter form`.

### Task 5: Set, If, Filter, Switch — schemas and the assignment/condition inputs

**Files:**
- Create: `src/n8n/schema/fields.rs`, `frontend/src/components/params/AssignmentsInput.vue`, `frontend/src/components/params/FilterInput.vue`, specs next to them
- Modify: `src/n8n/schema/mod.rs` (add `mod fields;` and try it in `for_node`), `ParameterInput.vue` (route `assignmentCollection` / `filter`)

**Interfaces:**
- Consumes: builder (Task 1), `ParameterInput` (Task 4).
- Produces: builder constructors `assignments(name, display)` → `type: "assignmentCollection"`, `default: {"assignments": []}`; `filter(name, display)` → `type: "filter"`, `default: {"conditions": [], "combinator": "and", "options": {"caseSensitive": true, "typeValidation": "strict"}}`.

- [ ] **Step 1: List what the nodes read** — `grep -nE '(param_[a-z0-9_]+|raw_param)\("' src/n8n/nodes/{set,conditions,routing}.rs`; note the assignment entry shape (`{id, name, value, type}`) and the condition shape (`{id, leftValue, rightValue, operator: {type, operation, singleValue?}}`) from `conditions.rs`.
- [ ] **Step 2: Write the failing frontend tests** — `AssignmentsInput.spec.ts`: adding a field emits `{assignments: [{id: <string>, name: 'chat_id', value: '={{ $json.message.chat.id }}', type: 'number'}]}`; `FilterInput.spec.ts`: choosing operator "is equal to" for type string emits `operator: {type: 'string', operation: 'equals'}`; an `exists` operator emits `singleValue: true` and no right value; switching the combinator emits `combinator: 'or'`.
- [ ] **Step 3: Run them to see them fail.**
- [ ] **Step 4: Implement** the two inputs (operators per type from `conditions.rs`: string — equals, notEquals, contains, notContains, startsWith, endsWith, regex, exists, notExists, empty, notEmpty; number — equals, notEquals, gt, lt, gte, lte, exists …; boolean — true, false, equals …; dateTime — after, before, equals …; array — contains, lengthEquals, empty …; object — exists, empty …), and the schemas for `set` (mode manual/raw, `assignments`, `jsonOutput` shown for raw, `includeOtherFields`, `include`, `options`), `if`/`filter` (`conditions` filter, `looseTypeValidation`, `options`), `switch` (mode rules/expression, `rules` fixed collection of `conditions` + output names, `numberOutputs`, `output`, `options`).
- [ ] **Step 5: Run all tests** — `cargo test --lib node_types:: && cd frontend && npx vitest run`; Expected: PASS.
- [ ] **Step 6: Commit** `feat: Set/If/Filter/Switch schemas with assignment and condition inputs`.

### Task 6: HTTP Request and Telegram schemas, and a BDD check against n8n

**Files:**
- Create: `src/n8n/schema/http.rs`, `src/n8n/schema/telegram.rs`
- Modify: `src/n8n/schema/mod.rs`, `tests/bdd/features/07-api/rest_auth_and_editor.feature`

- [ ] **Step 1: Add the BDD scenario** (it must pass on n8n too — it only checks fields both expose):

```gherkin
  Scenario: Node schemas describe real parameter types
    Given a running r8r server with an owner account
    When I send a GET request to "/types/nodes.json"
    Then the response status is 200
    And the response JSON at "" contains an element matching:
      """
      {"name": "n8n-nodes-base.code", "properties": [{"name": "language", "type": "options"}]}
      """
    And the response JSON at "" contains an element matching:
      """
      {"name": "n8n-nodes-base.limit", "properties": [{"name": "maxItems", "type": "number"}]}
      """
```

(`contains an element matching` with an array checks the listed entries are present, in `tests/bdd/support/json.rs` "subset" mode; if that mode isn't how arrays match, use one scenario step per property with `the response JSON at "<path>"`.)

- [ ] **Step 2: Run it on r8r** — `cargo test --test bdd -- -i 'tests/bdd/features/07-api/rest_auth_and_editor.feature'`; Expected: PASS (after Tasks 1–2).
- [ ] **Step 3: Run it on n8n 2.35.7** (README "Checking the expectations against n8n"); Expected: PASS. If it fails on n8n, the expectation is wrong — fix it.
- [ ] **Step 4: List what HTTP Request and Telegram read** — `grep -nE '(param_[a-z0-9_]+|raw_param)\("' src/n8n/nodes/{http,telegram}.rs`.
- [ ] **Step 5: Write the schemas.** HTTP Request: `method` (GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS; default GET), `url` (required), `authentication` (none, genericCredentialType, predefinedCredentialType), `sendQuery`/`queryParameters`, `sendHeaders`/`headerParameters`, `sendBody`/`contentType`/`bodyParameters`/`jsonBody` (each shown by its toggle), `options` (timeout, response format, redirects, …, from the reads). Telegram: `resource` (message, chat, callback, file), `operation` per resource (`.show("resource", …)`), message fields: `chatId`, `text`, `replyMarkup` (none, inlineKeyboard, replyKeyboard, replyKeyboardRemove, forceReply) and its keyboard fixed collections, `additionalFields` collection (parse mode, disable notification, disable web page preview, reply to message ID, message thread ID, protect content).
- [ ] **Step 6: Run** — `cargo test --lib node_types::`; Expected: PASS.
- [ ] **Step 7: Commit** `feat(n8n): HTTP Request and Telegram schemas; BDD check of schema types`.

---

## Done when

- `/types/nodes.json` serves real schemas for every node listed in Tasks 1, 2, 5 and 6; the coverage tests pass.
- `NodeParameters` renders and edits any of them (exercised by its tests).
- `07-api` passes on r8r and n8n 2.35.7.
- The roadmap's Phase 2 plan (remaining schemas) can start.
