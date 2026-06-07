import type { AgentState } from '../types/agent'
import { Play, Square, Trash2, Settings, GripVertical } from 'lucide-react'
import {
  DndContext, closestCenter, PointerSensor, useSensor, useSensors,
} from '@dnd-kit/core'
import type { DragEndEvent } from '@dnd-kit/core'
import {
  SortableContext, verticalListSortingStrategy, useSortable, arrayMove,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'

interface Props {
  agents: AgentState[]
  selectedId: string | null
  onSelect: (id: string) => void
  onStart: (id: string) => void
  onStop: (id: string) => void
  onDelete: (id: string) => void
  onConfigure: (agent: AgentState) => void
  onReorder: (newOrder: string[]) => void
}

function StatusDot({ status }: { status: AgentState['status'] }) {
  const cls: Record<string, string> = {
    running:  'bg-green-500',
    stopped:  'bg-gray-400 dark:bg-gray-600',
    error:    'bg-red-500',
    starting: 'bg-yellow-400 animate-pulse',
  }
  return <span className={`inline-block h-2 w-2 rounded-full shrink-0 ${cls[status] ?? 'bg-gray-400'}`} />
}

function SortableItem({
  agent, isSelected, onSelect, onStart, onStop, onDelete, onConfigure,
}: {
  agent: AgentState
  isSelected: boolean
  onSelect: () => void
  onStart: () => void
  onStop: () => void
  onDelete: () => void
  onConfigure: () => void
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: agent.config.id,
  })

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1 }}
      className={`group flex w-full items-center gap-2 rounded-xl px-2 py-2 transition-colors cursor-pointer ${
        isSelected
          ? 'border-l-2 border-blue-500 bg-blue-50 dark:bg-blue-600/10'
          : 'border-l-2 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
      }`}
      onClick={onSelect}
    >
      {/* Drag handle */}
      <button
        {...attributes}
        {...listeners}
        onClick={e => e.stopPropagation()}
        className="shrink-0 cursor-grab touch-none text-gray-300 opacity-0 group-hover:opacity-100 active:cursor-grabbing dark:text-gray-700"
      >
        <GripVertical className="h-4 w-4" />
      </button>

      {/* Avatar */}
      <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-sm font-semibold ${
        isSelected
          ? 'bg-blue-100 text-blue-600 dark:bg-blue-600/20 dark:text-blue-300'
          : 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400'
      }`}>
        {agent.config.name.charAt(0).toUpperCase()}
      </div>

      {/* Info */}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <StatusDot status={agent.status} />
          <span className="truncate text-sm font-medium text-gray-800 dark:text-gray-200">
            {agent.config.name}
          </span>
          {agent.config.port && (
            <span className={`shrink-0 rounded px-1 py-0.5 text-[10px] font-mono ${
              agent.port_open
                ? 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400'
                : 'bg-gray-100 text-gray-400 dark:bg-gray-800 dark:text-gray-500'
            }`}>
              :{agent.config.port}
            </span>
          )}
        </div>
        <p className="truncate text-xs text-gray-400 dark:text-gray-600">
          {agent.config.command} {agent.config.args.slice(0, 2).join(' ')}
        </p>
      </div>

      {/* Actions */}
      <div className="flex shrink-0 items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        {agent.status === 'running' ? (
          <ActionBtn onClick={onStop} title="Stop" danger>
            <Square className="h-3.5 w-3.5" />
          </ActionBtn>
        ) : (
          <ActionBtn onClick={onStart} title="Start">
            <Play className="h-3.5 w-3.5" />
          </ActionBtn>
        )}
        <ActionBtn onClick={onConfigure} title="Edit">
          <Settings className="h-3.5 w-3.5" />
        </ActionBtn>
        <ActionBtn onClick={onDelete} title="Delete" danger>
          <Trash2 className="h-3.5 w-3.5" />
        </ActionBtn>
      </div>
    </div>
  )
}

function ActionBtn({ children, onClick, title, danger }: {
  children: React.ReactNode
  onClick: () => void
  title: string
  danger?: boolean
}) {
  return (
    <button
      title={title}
      onClick={e => { e.stopPropagation(); onClick() }}
      className={`rounded p-1 transition-colors ${
        danger
          ? 'text-gray-400 hover:bg-red-100 hover:text-red-600 dark:text-gray-500 dark:hover:bg-red-900/30 dark:hover:text-red-400'
          : 'text-gray-400 hover:bg-gray-200 hover:text-gray-700 dark:text-gray-500 dark:hover:bg-gray-700 dark:hover:text-gray-200'
      }`}
    >
      {children}
    </button>
  )
}

export function AgentList({ agents, selectedId, onSelect, onStart, onStop, onDelete, onConfigure, onReorder }: Props) {
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = agents.findIndex(a => a.config.id === active.id)
    const newIndex = agents.findIndex(a => a.config.id === over.id)
    onReorder(arrayMove(agents, oldIndex, newIndex).map(a => a.config.id))
  }

  if (agents.length === 0) {
    return (
      <div className="flex flex-col items-center py-8">
        <p className="text-xs text-gray-400 dark:text-gray-600">No agents yet</p>
      </div>
    )
  }

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={agents.map(a => a.config.id)} strategy={verticalListSortingStrategy}>
        <div className="space-y-0.5 px-2 pb-2">
          {agents.map(agent => (
            <SortableItem
              key={agent.config.id}
              agent={agent}
              isSelected={selectedId === agent.config.id}
              onSelect={() => onSelect(agent.config.id)}
              onStart={() => onStart(agent.config.id)}
              onStop={() => onStop(agent.config.id)}
              onDelete={() => onDelete(agent.config.id)}
              onConfigure={() => onConfigure(agent)}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  )
}
