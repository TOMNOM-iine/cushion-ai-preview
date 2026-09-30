export type Phase = 'DRAFT' | 'SCANNING' | 'REVIEW' | 'APPROVED' | 'EXPORTED' | 'BLOCKED' | 'RESTORED_LOCAL';
export type Purpose = 'polish' | 'summary' | 'perspectives' | 'support_record';
export type Destination = 'ChatGPT' | 'Claude' | 'Gemini';
export type Mode = 'demo' | 'local';
export interface Finding { start: number; end: number; original: string; replacement: string; action: '置換' | '削除' | '一般化' | '停止'; reason: string; }
export interface Snapshot {
  caseId: string; revision: number; phase: Phase; original: string; purpose: Purpose;
  destination: Destination; mode: Mode; transformed: string; findings: Finding[];
  residual: string[]; lost: string[]; blockedReason: string | null;
  approvalId: string | null; jobId: string | null; restored: string;
  unknownTokens: string[]; testCopy: string | null; dictionaryText: string;
  policyVersion: string; modelConfig: string;
}
export interface CoreRequest { op: string; [key: string]: unknown; }
export interface ModelStatus {
  state: 'unavailable' | 'verifying' | 'ready' | 'busy' | 'error';
  modelName: string;
  modelConfig: string | null;
  reason: string | null;
  contextTokens: number | null;
  sourceVerified: boolean;
  networkIsolated: boolean;
}
export interface VaultStatus { available: boolean; unlocked: boolean; reason: string | null; }
export interface VaultEntry { id: string; createdMs: number; bytes: number; }
export type ApiProvider = 'OpenAI' | 'Claude';
export interface ApiStatus { configuredOpenAI: boolean; configuredClaude: boolean; }
export type DeliveryMode = 'api_key' | 'chatgpt_plan';
export interface SubscriptionStatus {
  state: 'disconnected' | 'connecting' | 'connected' | 'error';
  reason: string | null;
  model: string | null;
  models: { slug: string; displayName: string }[];
  deliveryMode: DeliveryMode;
}
