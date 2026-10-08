# Editor on the n8n Engine — Roadmap

**Goal:** the Vue editor runs workflows on the n8n-compatible engine (`src/n8n`, ~75 node types) instead of the legacy engine (`src/nodes`, 16 types), with a form for every node's parameters.

**Why:** the spec's ~60 native nodes and the AI cluster already exist in `src/n8n` (1001 BDD scenarios against real n8n). The editor was built on the legacy engine, so each request for a node or field meant re-implementing it there. One engine ends that.

**Spec:** `raw/2026-09-25-n8n-in-rust-reimplementation-spec.md` — §4.3 ("Node description as data … the single most important contract"), §6.6 (native node list), §6.7 (Code), §6.8 (AI), §9 (licensing: no copying of n8n node descriptions without legal sign-off).

Each phase is its own plan, written when the previous one ships, and each ends in working, tested software.

| Phase | Plan | Delivers |
| --- | --- | --- |
| 1 | `2026-10-08-node-property-schemas.md` | Clean-room property schemas (n8n `INodeTypeDescription` shape) for the core nodes, served at `/types/nodes.json`; a generic schema-driven parameter form in the editor, tested in isolation. |
| 2 | node schemas, batch 2 | Schemas for the remaining base nodes (data transforms, files, databases, messaging, Google/GitHub/Notion …) and the AI cluster (agents, chains, models, memories, tools, vector stores) incl. `ai_*` connection types. A coverage test fails for any native node without a real schema. |
| 3 | editor data model | The editor reads and writes n8n workflow JSON through `/rest/workflows` (nodes by name + `typeVersion`, connections by name incl. `ai_*`), renders sub-node connections (the AI Agent's model/memory/tool ports become real sub-nodes), uses the Phase 1 form for every node, keeps auto-apply, delete/insert, start/end shapes. |
| 4 | live runs on the n8n engine | Manual run, stop, Active switch, run colours and "N items" from `/rest/push` (`nodeExecuteBefore/After`, `executionFinished`) and run data; results panel from run data. |
| 5 | parity gaps in the n8n engine | Telegram Trigger for the n8n engine (webhook + local polling mode, test events, "Trigger on"), `Intl` in the n8n engine's VM (Luxon-compatible: `formatToParts`, time zones), anything else the legacy editor offered that the n8n engine lacks. |
| 6 | migration and retirement | Converter for existing editor workflows (legacy → n8n JSON, incl. credentials and memory keys), run at startup with a report; remove the `/rest/r8r` editor endpoints and `src/nodes`. |

## Checkpoint rule

Every plan task is ≤ 20 minutes and ends with the `=== CHECKPOINT ===` block (completed, data, next).
