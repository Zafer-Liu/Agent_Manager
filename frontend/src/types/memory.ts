/** Agent Manager 内置本地语义索引类型定义 */

export interface MemoryConfig {
  /** Agent Manager 私有本机 sidecar 地址（仅内部使用） */
  baseUrl: string
  /** API Key（Bearer 认证） */
  apiKey: string
  /** 记忆归属用户 ID */
  userId: string
}

/** get / search 返回的记忆条目 */
export interface MemoryItem {
  id: string
  memory: string
  memory_type?: string
  durability?: 'session' | 'short_term' | 'long_term'
  last_update_at?: string
  event_time?: string | null
  score?: number | null
  retrieval_source?: 'memory' | 'conversation' | 'local'
  retrieval_method?: 'semantic' | 'keyword' | 'local'
  session_id?: string
  source_agent?: string
  occurred_at?: string
}

export interface MemoryImportance {
  memory_id: string
  score: number
  supporting_sessions: number
  supporting_agents: number
  duplicate_count: number
  recall_count: number
  pinned: boolean
  updated_at: string
}

export interface LocalMemoryStats {
  total: number
  facts: number
  preferences: number
  session_only: number
  short_term: number
  long_term: number
}

export interface L1ResetResult {
  cleared_l1: number
  cleared_derived_documents: number
  requeued_conversations: number
}

export interface MemoryImportanceSummary {
  reviewed: number
  high: number
  medium: number
  low: number
  message: string
}

export type MemorySearchMode = 'strict' | 'balanced' | 'broad'

export interface ConversationSearchHit {
  id: number
  source: string
  session_id: string
  occurred_at: string
  score: number
  preview: string
}

/** Global overview distilled from completed conversations across all Agents. */
export interface WorkspaceSummary {
  workspace_id: string
  content: string
  source_event_count: number
  updated_at: string
}

export interface ProfileSummary {
  content: string
  source_workspace_count: number
  updated_at: string
}

export interface MemoryLayerDocument {
  id: string
  layer: 'l2' | 'l3'
  scope: string
  content: string
  state: 'draft' | 'published' | 'archived' | 'failed'
  token_estimate: number
  source_count: number
  window_start?: string | null
  window_end?: string | null
  created_at: string
  published_at?: string | null
}

export interface MemoryLayerRunResult {
  document: MemoryLayerDocument
  selected_l1_count: number
  stage_count: number
  message: string
}

/** add 返回的抽取事件条目 */
export interface AddEventItem {
  operation?: string
  memory_id?: string
  content?: string
}

export interface AddResult {
  memories: AddEventItem[]
}

export interface MemoryListResult {
  memories: MemoryItem[]
}

/** 内置记忆引擎组件在线状态（来自 Tauri 后端） */
export interface ComponentStatus {
  name: string
  online: boolean
  detail: string
}

export interface EngineStatus {
  online: boolean
  api_port: number
  components: ComponentStatus[]
}

export interface ConsolidationResult {
  snapshot_id: string
  before_count: number
  after_count: number
  scopes: number
  clusters: number
  actions: number
  message: string
}

/** A locally selected BGE semantic-neighbour batch for safe LLM consolidation. */
export interface ConsolidationCandidate {
  id: string
  memory: string
  memory_type?: string
}

export type HealthState = 'unknown' | 'online' | 'offline'

/** 自动沉淀管道（hook 回调）相关类型 */
export interface IngestLog {
  at: string
  agent_id: string
  kind: 'memory' | 'skill'
  state: 'working' | 'stored' | 'retrying'
  detail: string
}

export interface HookStatus {
  installed: boolean
  agent_type: string
  events: string[]
}

/** User-level shared-memory MCP installation status for an external Agent. */
export interface MemoryMcpStatus {
  agent_type: MemoryMcpTarget
  installed: boolean
  executable: string
  detail: string
}

export type MemoryMcpTarget = 'codex_cli' | 'claude_cli' | 'codex_desktop' | 'claude_desktop' | 'qoder' | 'workbuddy'

export interface IngestStatus {
  enabled: boolean
  buffered_sessions: number
  model_provider_id: string | null
  model_ready: boolean
  recent: IngestLog[]
}

/** Result of importing a user-selected folder of exported Agent memories. */
export interface MemoryImportResult {
  folder: string
  scanned_files: number
  recognized_files: number
  skipped_files: number
  imported_memories: number
  message: string
}

export interface OrganizeConversationsResult {
  attempted: number
  succeeded: number
  failed: number
  failure_reasons: string[]
}

/** A privacy-preserving summary of one external Agent MCP request. */
export interface McpAccessLog {
  id: number
  occurred_at: string
  client_name: string
  tool_name: string
  summary: string
  success: boolean
}

export interface TelemetrySummary {
  events: number
  sessions: number
  input_tokens: number
  output_tokens: number
  cached_tokens: number
  total_tokens: number
  reported_events: number
  estimated_events: number
  usage_sessions: number
  unavailable_usage_sessions: number
  usage_refreshed_at: string | null
}

export interface TelemetryUsageRefresh {
  scanned_sessions: number
  updated_sessions: number
  unavailable_sessions: number
  message: string
}

export interface TelemetryUsageBucket {
  label: string
  input_tokens: number
  output_tokens: number
  cached_tokens: number
  record_count: number
}

export interface TelemetryUsageAnalytics {
  record_count: number
  input_tokens: number
  output_tokens: number
  cached_tokens: number
  records: TelemetryUsageRecord[]
  truncated_records: boolean
  buckets: TelemetryUsageBucket[]
  sources: string[]
}

/** Live local-ledger state, refreshed by the Memory Center polling loop. */
export interface TelemetryLiveStatus {
  captured_sessions: number
  active_sessions: number
  completed_conversations: number
  organized_memory_conversations: number
  pending_memory_sessions: number
  retrying_memory_sessions: number
  failed_memory_sessions: number
  pending_usage_sessions: number
  failed_transcript_scans: number
  last_event_at: string | null
}

/** Durable local hook record.  It is retained even when memory extraction is offline. */
export interface TelemetryEvent {
  id: number
  source: string
  session_id: string
  event_type: string
  occurred_at: string
  token_source: 'reported' | 'estimated' | 'unavailable'
  input_tokens: number
  output_tokens: number
  cached_tokens: number
  conversation_state: 'full' | 'partial' | 'unavailable'
  conversation_message_count: number
  conversation_text: string | null
  l1_state: 'unavailable' | 'pending' | 'stored' | 'retrying'
}

/** A request/response observation reconstructed from a native transcript.
 * `session_total` is an explicit fallback for agents that do not expose
 * request-level provider counters. */
export interface TelemetryUsageRecord {
  record_id: string
  source: string
  session_id: string
  occurred_at: string
  model: string | null
  record_kind: 'response' | 'session_total'
  input_tokens: number
  output_tokens: number
  cached_tokens: number
  origin: string
}

export interface SkillItem {
  source: string
  name: string
  description: string
  path: string
  hash: string
  version: number
  status: 'draft' | 'published'
  assigned_agents: string[]
}

export interface SkillSyncPreview {
  target: string
  create: SkillItem[]
  update: SkillItem[]
  unchanged: SkillItem[]
  conflict: SkillItem[]
}

export interface SkillDocument {
  item: SkillItem
  content: string
}
