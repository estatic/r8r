<script lang="ts">
import type { Connection } from '../types/domain'

export function edgeSourceHandle(c: Connection): string {
  return c.error ? 'error' : String(c.from_output)
}

export function edgeStyle(c: Connection): { stroke: string } | undefined {
  return c.error ? { stroke: '#dc2626' } : undefined
}

export function connectionFromVueFlow(connection: {
  source: string
  sourceHandle?: string | null
  target: string
  targetHandle?: string | null
}): Connection {
  const isError = connection.sourceHandle === 'error'
  return {
    from_node: connection.source,
    from_output: isError ? 0 : Number(connection.sourceHandle ?? 0),
    to_node: connection.target,
    to_input: Number(connection.targetHandle ?? 0),
    error: isError,
  }
}
</script>

<script setup lang="ts">
import { computed, reactive, ref, watch, nextTick } from 'vue'
import { VueFlow, Handle, Position, MarkerType, useVueFlow, type Node as FlowNode, type Edge as FlowEdge } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import type { Execution, NodeInstance } from '../types/domain'
import { useNodeTypesStore } from '../stores/nodeTypes'
import { useToolsStore } from '../stores/tools'
import { agentSetupProblems, describeSetupProblems } from '../agent/setup'
import { RUN_COLORS, edgeRunState, itemsLabel, nodeRunState, nodeShape } from '../canvas/runView'
import RunEdge from './RunEdge.vue'

const props = defineProps<{
  nodes: NodeInstance[]
  connections: Connection[]
  /** The run on screen: colours nodes and links, and counts items. */
  execution?: Execution | null
}>()

const emit = defineEmits<{
  'node-select': [nodeId: string]
  'node-move': [nodeId: string, position: [number, number]]
  connect: [connection: Connection]
  'edge-insert': [connection: Connection]
  'edge-delete': [connection: Connection]
  /** A node's auxiliary port (an AI Agent's chat model, memory or tool) was clicked. */
  'aux-open': [nodeId: string, port: AuxPort]
}>()

type AuxPort = 'model' | 'memory' | 'tools'

/** The AI Agent's auxiliary inputs, drawn below it like n8n's sub-node ports. */
function auxPorts(n: NodeInstance): { key: AuxPort; label: string; required: boolean; set: boolean; detail?: string; glow?: boolean }[] {
  if (n.node_type !== 'ai.agent') return []
  const p = n.parameters ?? {}
  const memory = p.memory as { enabled?: boolean } | undefined
  const tools = (p.tool_ids as unknown[] | undefined) ?? []
  return [
    // The chosen model is named under its port, which glows green.
    ...[typeof p.model === 'string' ? p.model.trim() : ''].map((model) => ({
      key: 'model' as const,
      label: 'chat model',
      required: true,
      set: model !== '',
      detail: model || undefined,
      glow: model !== '',
    })),
    { key: 'memory', label: 'memory', required: false, set: memory?.enabled === true },
    { key: 'tools', label: 'tool', required: false, set: tools.length > 0 },
  ]
}

/** Why the node shows its red "!": unfinished setup, or its failed last run. */
function problemOf(n: NodeInstance): string | null {
  const setup = agentSetupProblems([n])
  if (setup.length > 0) return describeSetupProblems(setup)
  const run = props.execution?.node_runs?.[n.id]
  if (run?.status === 'error') {
    const message = (props.execution?.node_outputs[n.id]?.[0]?.json as Record<string, unknown> | undefined)?.error
    return `The last run failed here${typeof message === 'string' ? `: ${message}` : '.'}`
  }
  return null
}

const { onConnect, onNodeDragStop, onNodeClick, onEdgeMouseEnter, onEdgeMouseLeave, updateNodeInternals, viewport, dimensions } = useVueFlow()

/**
 * Where a new node goes: the middle of what is on screen, stepped down and
 * right past any node already there, so it is always in view.
 */
function freeSpot(): [number, number] {
  const { x, y, zoom } = viewport.value
  const { width, height } = dimensions.value
  // Before the canvas has a size (tests, first paint), fall back to the origin area.
  let px = width > 0 ? Math.round((width / 2 - x) / zoom - NODE_SIZE.w / 2) : 100
  let py = height > 0 ? Math.round((height / 2 - y) / zoom - NODE_SIZE.h / 2) : 100
  const taken = (ax: number, ay: number) => props.nodes.some((n) => Math.abs(n.position[0] - ax) < NODE_SIZE.w && Math.abs(n.position[1] - ay) < NODE_SIZE.h + 20)
  for (let i = 0; i < 30 && taken(px, py); i++) {
    px += 40
    py += 40
  }
  return [px, py]
}
defineExpose({ freeSpot })

// The hovered link shows its "+" / delete buttons; leaving the line for the
// buttons (or back) must not hide them, hence the short grace period.
const hoveredEdgeId = ref<string | null>(null)
let unhoverTimer: ReturnType<typeof setTimeout> | null = null
function hoverEdge(id: string, on: boolean) {
  if (unhoverTimer) clearTimeout(unhoverTimer)
  if (on) hoveredEdgeId.value = id
  else unhoverTimer = setTimeout(() => (hoveredEdgeId.value = null), 250)
}
onEdgeMouseEnter(({ edge }) => hoverEdge(edge.id, true))
onEdgeMouseLeave(({ edge }) => hoverEdge(edge.id, false))

/** About a node's footprint on the canvas, for placing new ones. */
const NODE_SIZE = { w: 180, h: 60 }

const nodeTypesStore = useNodeTypesStore()
if (!nodeTypesStore.loaded) {
  nodeTypesStore.fetchAll().catch(() => {})
}

const toolsStore = useToolsStore()
if (!toolsStore.loaded) {
  toolsStore.fetchAll().catch(() => {})
}

function metaFor(nodeType: string) {
  return nodeTypesStore.types.find((t) => t.type_name === nodeType)
}

function labelFor(nodeType: string): string {
  const meta = metaFor(nodeType)
  return meta ? `${meta.icon} ${meta.display_name}` : nodeType
}

/** The AI Agent's tools (picked in its panel), shown hanging below it. */
function toolNames(n: NodeInstance): string[] {
  if (n.node_type !== 'ai.agent') return []
  const ids = (n.parameters?.tool_ids as string[] | undefined) ?? []
  return ids.map((id) => toolsStore.tools.find((t) => t.id === id)?.name ?? 'tool')
}

// nodeId -> live output port labels for that node's current parameters.
// Populated by watching props.nodes and re-fetching only when a given
// node's (node_type, parameters) pair actually changes -- not per
// keystroke, since NodeConfigPanel only propagates parameter edits on
// Apply.
const portsByNodeId = reactive<Record<string, string[]>>({})
const lastFetchedKey = new Map<string, string>()

watch(
  () => props.nodes,
  (nodes) => {
    for (const n of nodes) {
      const key = `${n.node_type}:${JSON.stringify(n.parameters)}`
      if (lastFetchedKey.get(n.id) === key) continue
      lastFetchedKey.set(n.id, key)
      nodeTypesStore
        .portsFor(n.node_type, n.parameters)
        .then((ports) => {
          portsByNodeId[n.id] = ports
          nextTick(() => updateNodeInternals([n.id]))
        })
        .catch(() => {
          portsByNodeId[n.id] = ['main']
          nextTick(() => updateNodeInternals([n.id]))
        })
    }
  },
  { immediate: true, deep: true },
)

const flowNodes = computed<FlowNode[]>(() =>
  props.nodes.map((n) => ({
    id: n.id,
    position: { x: n.position[0], y: n.position[1] },
    label: labelFor(n.node_type),
    data: {
      nodeType: n.node_type,
      icon: metaFor(n.node_type)?.icon ?? '',
      name: metaFor(n.node_type)?.display_name ?? n.node_type,
      // Nodes with ports below them stay boxes, so the ports have an edge to hang from.
      shape: n.node_type === 'ai.agent' ? 'box' : nodeShape(n.id, metaFor(n.node_type)?.category, props.connections),
      aux: auxPorts(n),
      problem: problemOf(n),
      run: nodeRunState(props.execution ?? null, n.id),
      tools: toolNames(n),
      disabled: n.disabled,
      outputPorts: portsByNodeId[n.id] ?? ['main'],
      needsSetup: agentSetupProblems([n]).length > 0,
    },
  })),
)

const flowEdges = computed<FlowEdge[]>(() =>
  props.connections.map((c) => {
    const state = edgeRunState(props.execution ?? null, c)
    const color = state ? RUN_COLORS[state] : edgeStyle(c)?.stroke
    const id = `${c.from_node}:${c.error ? 'error' : c.from_output}->${c.to_node}:${c.to_input}`
    return {
      id,
      type: 'run',
      source: c.from_node,
      target: c.to_node,
      sourceHandle: edgeSourceHandle(c),
      targetHandle: String(c.to_input),
      markerEnd: { type: MarkerType.ArrowClosed, color },
      animated: state === 'running',
      style: color ? { stroke: color, strokeWidth: state && state !== 'pending' ? 2 : 1 } : undefined,
      data: {
        itemsLabel: itemsLabel(props.execution ?? null, c),
        color,
        hovered: hoveredEdgeId.value === id,
        onHover: (on: boolean) => hoverEdge(id, on),
        onInsert: () => {
          hoveredEdgeId.value = null
          emit('edge-insert', c)
        },
        onDelete: () => {
          hoveredEdgeId.value = null
          emit('edge-delete', c)
        },
      },
    }
  }),
)

function glow(run: string | null): Record<string, string> {
  if (!run) return {}
  const c = RUN_COLORS[run as keyof typeof RUN_COLORS]
  return { filter: `drop-shadow(0 0 3px ${c}) drop-shadow(0 0 6px ${c})` }
}

onNodeClick((event) => {
  emit('node-select', event.node.id)
})

onNodeDragStop((event) => {
  emit('node-move', event.node.id, [event.node.position.x, event.node.position.y])
})

onConnect((connection) => {
  emit('connect', connectionFromVueFlow(connection))
})

function handlePosition(index: number, total: number): string {
  return `${((index + 1) * 100) / (total + 1)}%`
}

/** Output handle `index` of `total` down the right side; an end's sit together at its tip. */
function outputTop(shape: string, index: number, total: number): string {
  if (shape === 'end') return `calc(50% + ${(index - (total - 1) / 2) * 16}px)`
  return handlePosition(index, total)
}
</script>

<template>
  <div class="w-full h-full">
    <VueFlow :nodes="flowNodes" :edges="flowEdges" fit-view-on-init :delete-key-code="null">
      <template #edge-run="edgeProps">
        <RunEdge v-bind="edgeProps" />
      </template>
      <template #node-default="{ id, data, label }">
        <div
          class="relative"
          :class="{ 'opacity-50': data.disabled }"
          :data-shape="data.shape"
          :data-run="data.run ?? undefined"
          :title="data.nodeType"
        >
          <Handle
            v-if="data.shape !== 'start'"
            id="0"
            type="target"
            :position="Position.Left"
            class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white"
          />
          <!-- A start is a triangle with its flat side right (where it sends), an end one with its flat side left. -->
          <div v-if="data.shape !== 'box'" class="relative w-16 h-14" :style="glow(data.run)">
            <svg viewBox="0 0 64 56" class="absolute inset-0 w-full h-full">
              <polygon
                :points="data.shape === 'start' ? '2,28 62,2 62,54' : '2,2 2,54 62,28'"
                class="fill-white stroke-gray-400"
                stroke-width="1.5"
                stroke-linejoin="round"
              />
            </svg>
            <span
              class="absolute top-1/2 -translate-y-1/2 text-lg"
              :class="data.shape === 'start' ? 'right-2.5' : 'left-2.5'"
              >{{ data.icon }}</span
            >
            <div class="absolute top-full mt-1 left-1/2 -translate-x-1/2 whitespace-nowrap text-xs text-gray-700">
              {{ data.name }}
            </div>
          </div>
          <div
            v-else
            class="px-3 py-2 rounded border bg-white shadow text-xs"
            :class="{ 'min-w-[220px] py-3 text-center': data.aux.length > 0 }"
            :style="{ ...glow(data.run), minHeight: `${(data.outputPorts.length + 2) * 14}px` }"
          >
            {{ label }}
          </div>
          <!-- Not set up, or its last run failed: a small red "!" in the corner, the reason on hover. -->
          <span
            v-if="data.problem"
            data-testid="needs-setup"
            :title="data.problem"
            class="absolute -bottom-1.5 -right-1.5 z-10 w-4 h-4 rounded-sm bg-red-600 text-white text-[10px] font-bold leading-4 text-center shadow"
            >!</span
          >
          <Handle
            v-for="(port, i) in data.outputPorts"
            :key="port"
            :id="String(i)"
            type="source"
            :position="Position.Right"
            :style="{ top: outputTop(data.shape, i, data.outputPorts.length + 1) }"
            class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white"
          />
          <div
            v-for="(port, i) in data.outputPorts"
            v-show="!(data.outputPorts.length === 1 && port === 'main')"
            :key="`label-${port}`"
            class="absolute text-[10px] text-gray-600 left-full ml-2"
            :style="{ top: outputTop(data.shape, i, data.outputPorts.length + 1), transform: 'translateY(-50%)' }"
          >
            {{ port }}
          </div>
          <Handle
            id="error"
            type="source"
            :position="Position.Right"
            :style="{ top: outputTop(data.shape, data.outputPorts.length, data.outputPorts.length + 1) }"
            class="w-2.5 h-2.5 rounded-full bg-red-500 border border-white"
          />
          <div
            class="absolute text-[10px] text-red-600 left-full ml-2"
            :style="{ top: outputTop(data.shape, data.outputPorts.length, data.outputPorts.length + 1), transform: 'translateY(-50%)' }"
          >
            error
          </div>
          <!-- Auxiliary inputs (an AI Agent's chat model, memory, tools) hang below the node. -->
          <div v-if="data.aux.length > 0" class="absolute top-full left-0 right-0 flex justify-around">
            <div
              v-for="port in data.aux"
              :key="port.key"
              :data-testid="`aux-port-${port.key}`"
              :data-set="String(port.set)"
              class="flex flex-col items-center"
            >
              <div class="h-5 w-px bg-gray-800" />
              <button
                type="button"
                :title="port.set ? `Change the ${port.label}` : `Set up the ${port.label}`"
                class="group nodrag nopan relative -mt-px w-3.5 h-3.5 rounded-full border border-gray-700 flex items-center justify-center transition-all hover:w-5 hover:h-5 hover:bg-blue-600 hover:border-blue-600"
                :class="port.set ? 'bg-gray-700' : 'bg-white'"
                :style="port.glow ? { boxShadow: '0 0 0 2px #fff, 0 0 6px 3px #22c55e' } : undefined"
                @click.stop="emit('aux-open', id, port.key)"
              >
                <span class="hidden group-hover:block text-white text-sm leading-none font-bold">+</span>
              </button>
              <span data-testid="aux-label" class="mt-0.5 whitespace-nowrap text-[10px] text-gray-600"
                >{{ port.label }}<span v-if="port.required" data-testid="aux-required" class="text-red-600">*</span></span
              >
              <span
                v-if="port.detail"
                data-testid="aux-detail"
                :title="port.detail"
                class="max-w-[110px] truncate text-[10px] font-medium text-gray-800"
                >{{ port.detail }}</span
              >
              <div v-if="port.key === 'tools' && data.tools.length > 0" data-testid="agent-tools" class="mt-1 flex flex-col items-center gap-1">
                <span
                  v-for="(tool, i) in data.tools"
                  :key="i"
                  class="whitespace-nowrap text-[10px] px-1.5 py-0.5 rounded-full border border-dashed border-gray-400 bg-white text-gray-700"
                  >🔧 {{ tool }}</span
                >
              </div>
            </div>
          </div>
        </div>
      </template>
    </VueFlow>
  </div>
</template>
