/** Local IPC data. A Snapshot is NEVER a provider request. */
export type Purpose = 'polish' | 'summary' | 'perspectives' | 'support_record';
export type Destination = 'ChatGPT' | 'Claude' | 'Gemini';
export type Phase = 'DRAFT' | 'SCANNING' | 'REVIEW' | 'APPROVED' | 'EXPORTED' | 'BLOCKED' | 'RESTORED_LOCAL';
export interface Finding {
  start: number; end: number; original: string; replacement: string;
  action: '置換' | '削除' | '一般化' | '停止'; reason: string;
}
export interface Snapshot {
  caseId: string; revision: number; phase: Phase; original: string;
  purpose: Purpose; destination: Destination; mode: 'demo' | 'local';
  transformed: string; findings: Finding[]; residual: string[]; lost: string[];
  blockedReason: string | null; approvalId: string | null; jobId: string | null;
  restored: string; unknownTokens: string[]; testCopy: string | null;
  dictionaryText: string; policyVersion: string; modelConfig: string;
}
export interface CoreRequest { op: string; [key: string]: unknown }
export type Dispatch = (request: CoreRequest) => Promise<Snapshot>;
export interface PrepareInput { original: string; purpose: Purpose; destination: Destination }

/** Legacy callback shape for type compatibility; arbitrary transports are disabled. */
export interface OutboundTransportRequest { readonly destination: Destination; readonly text: string }
export interface ProviderTextResponse { readonly text: string }
export type ApprovedTransport = (request: OutboundTransportRequest) => Promise<ProviderTextResponse>;
/** Trusted local native IPC capability; sends an approval ID, never a request body. */
export interface LocalOnlyHost {
  readonly isNative: true;
  sendApproved(approvalId: string): Promise<Snapshot>;
}
export interface DemoRequestPreview {
  readonly mode: 'demo';
  readonly destination: Destination;
  readonly method: 'SIMULATED';
  readonly endpoint: null;
  readonly body: { readonly text: string };
  readonly externalRequestMade: false;
}
export interface DemoRoundTrip {
  snapshot: Snapshot;
  outboundText: string;
  providerAnswer: string;
  restored: string;
  requestPreview: DemoRequestPreview;
}
