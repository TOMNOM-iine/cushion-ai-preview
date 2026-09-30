import type {
  ApprovedTransport, CoreRequest, DemoRoundTrip, Dispatch,
  PrepareInput, Snapshot, LocalOnlyHost,
} from './contracts';
export type * from './contracts';

export type CushionErrorCode = 'invalid-input' | 'core-rejected' | 'stale-result' | 'review-required' | 'demo-only' | 'busy' | 'production-unavailable';
export class CushionError extends Error {
  readonly code: CushionErrorCode;
  constructor(code: CushionErrorCode, message: string) {
    super(message); this.name = 'CushionError'; this.code = code;
  }
}
interface Context { caseId: string; revision: number }
interface ApprovalContext extends Context { approvalId: string }

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype;
}
function validateInput(value: unknown): asserts value is PrepareInput {
  if (!isPlainObject(value)
      || Object.keys(value).some(key => !['original', 'purpose', 'destination'].includes(key))
      || typeof value.original !== 'string'
      || !['polish', 'summary', 'perspectives', 'support_record'].includes(String(value.purpose))
      || !['ChatGPT', 'Claude', 'Gemini'].includes(String(value.destination))) {
    throw new CushionError('invalid-input', '原文・目的・送信予定先だけを指定してください。会話履歴や追加の指示・ツールをそのまま渡すことはできません。');
  }
}
function sameContext(snapshot: Snapshot, context: Context): boolean {
  return snapshot.caseId === context.caseId && snapshot.revision === context.revision;
}
function stale(): never {
  throw new CushionError('stale-result', '確認中に文章・設定・相談が変わりました。古い結果は採用していません。現在の文章をもう一度確認してください。');
}

/**
 * SDK for an existing local chat UI. `dispatch` must be trusted local Tauri IPC
 * or the synthetic WASM adapter, never an HTTP bridge for original documents.
 * No API key, fetch, history, remote callback, logging, or storage is used here.
 */
export function createCushionClient({ dispatch, localOnlyHost }: { dispatch: Dispatch; localOnlyHost?: LocalOnlyHost }) {
  if (typeof dispatch !== 'function') throw new CushionError('invalid-input', 'ローカルのCore接続が必要です。');
  let generation = 0;
  let reviewContext: Context | null = null;
  let approvalContext: ApprovalContext | null = null;
  let roundTripRunning = false;

  async function call(request: CoreRequest): Promise<Snapshot> {
    try { return await dispatch(request); }
    catch {
      // Host exceptions may contain sensitive data. Never echo them into UI/logs.
      throw new CushionError('core-rejected', 'Coreがこの操作を許可しませんでした。原文を保持したまま、最新の変換結果とモデルの準備状態を確認してください。');
    }
  }
  async function current(context: Context, run: number): Promise<Snapshot> {
    const snapshot = await call({ op: 'snapshot' });
    if (run !== generation || !sameContext(snapshot, context)) stale();
    return snapshot;
  }

  async function prepare(input: PrepareInput): Promise<Snapshot> {
    validateInput(input);
    const run = ++generation;
    reviewContext = null; approvalContext = null;
    // Construct a fresh narrow object: even a caller with `any` cannot smuggle
    // system/history/messages/tools/retrieval fields through this boundary.
    const updated = await call({ op: 'update', patch: {
      original: input.original, purpose: input.purpose, destination: input.destination,
    } });
    let context = { caseId: updated.caseId, revision: updated.revision };
    if (run !== generation) stale();
    if (updated.phase === 'BLOCKED') return updated;
    if (updated.original !== input.original || updated.purpose !== input.purpose || updated.destination !== input.destination) {
      stale();
    }
    await current(context, run);
    const scanning = await call({ op: 'begin_scan' });
    if (localOnlyHost?.isNative === true && updated.mode === 'local') {
      // A trusted host may bind its measured model/policy to this job and advance
      // the revision while starting it. The original input and case must match.
      if (run !== generation || scanning.caseId !== context.caseId || scanning.mode !== 'local'
          || scanning.original !== input.original || scanning.purpose !== input.purpose
          || scanning.destination !== input.destination || scanning.revision < context.revision) stale();
      context = { caseId: scanning.caseId, revision: scanning.revision };
      let state = scanning;
      const deadline = performance.now() + 180_000;
      while (state.phase === 'SCANNING') {
        if (performance.now() >= deadline) throw new CushionError('core-rejected', 'ローカル検査の完了を待てませんでした。原文は保持しています。処理状態を確認し、必要ならキャンセルしてください。');
        // Real state polling, not an artificial animation delay. The native worker
        // alone completes this job; the SDK never fabricates finish_scan output.
        await new Promise(resolve => setTimeout(resolve, 100));
        state = await current(context, run);
      }
      const latest = await current(context, run);
      if (latest.phase !== state.phase) stale();
      if (latest.phase === 'REVIEW') reviewContext = context;
      else if (latest.phase !== 'BLOCKED') stale();
      return latest;
    }
    if (run !== generation || !sameContext(scanning, context)) stale();
    if (scanning.phase === 'BLOCKED') return scanning;
    if (scanning.phase !== 'SCANNING' || !scanning.jobId) {
      throw new CushionError('review-required', '確認を開始できませんでした。現在の文章からやり直してください。');
    }
    await current(context, run);
    const reviewed = await call({ op: 'finish_scan', jobId: scanning.jobId, fault: 'none' });
    if (run !== generation || !sameContext(reviewed, context)) stale();
    const latest = await current(context, run);
    if (latest.phase !== reviewed.phase) stale();
    if (reviewed.phase === 'REVIEW') reviewContext = context;
    else if (reviewed.phase !== 'BLOCKED') stale();
    return latest;
  }

  async function approve(reviewRevision: number): Promise<string> {
    const context = reviewContext; const run = generation;
    if (!context || context.revision !== reviewRevision || !Number.isSafeInteger(reviewRevision)) {
      throw new CushionError('review-required', '最新の原文と変換文を確認してから承認してください。');
    }
    const before = await current(context, run);
    if (before.phase !== 'REVIEW') throw new CushionError('review-required', 'この文章はまだ承認できません。最新の確認画面に戻ってください。');
    const approved = await call({ op: 'approve', revision: reviewRevision });
    if (run !== generation || !sameContext(approved, context)) stale();
    if (approved.phase !== 'APPROVED' || !approved.approvalId) {
      throw new CushionError('review-required', '承認を確認できませんでした。送信していません。');
    }
    const latest = await current(context, run);
    if (latest.phase !== 'APPROVED' || latest.approvalId !== approved.approvalId) stale();
    approvalContext = { ...context, approvalId: approved.approvalId };
    return approved.approvalId;
  }

  async function runDemo(approvalId: string): Promise<DemoRoundTrip> {
    if (roundTripRunning) throw new CushionError('busy', 'この相談のデモ応答を準備しています。');
    const context = approvalContext; const run = generation;
    if (!context || context.approvalId !== approvalId) {
      throw new CushionError('review-required', 'この相談で確認した変換文を承認してから、デモを実行してください。');
    }
    roundTripRunning = true;
    try {
      const before = await current(context, run);
      if (before.mode !== 'demo') throw new CushionError('demo-only', 'この体験は架空データのデモ専用です。');
      if (before.phase !== 'APPROVED' || before.approvalId !== approvalId) stale();
      const exported = await call({ op: 'demo_export', approvalId });
      if (run !== generation || !sameContext(exported, context)) stale();
      if (exported.phase !== 'EXPORTED' || !exported.testCopy || exported.testCopy !== exported.transformed) {
        throw new CushionError('core-rejected', '承認済みのデモ出力を確認できませんでした。外部送信はしていません。');
      }
      // Synthetic responder gets ONLY what the Core allowed into its test port.
      // It is fixed local code, not an injectable provider/transport callback.
      const outboundText = exported.testCopy;
      const tokens = [...new Set(outboundText.match(/\[\[CUSHION_[0-9a-f]{32}\]\]/g) ?? [])];
      const providerAnswer = '【外部通信のない固定デモ回答】\n\n確認した仮名をそのまま残した回答例です。\n'
        + tokens.join('、') + '\n\nこれは接続手順を試す合成回答です。実際のAIによる文章生成は行っていません。';
      const awaiting = await current(context, run);
      if (awaiting.phase !== 'EXPORTED' || awaiting.approvalId !== approvalId) stale();
      const snapshot = await call({ op: 'restore_if_current', answer: providerAnswer,
        caseId: context.caseId, revision: context.revision });
      if (run !== generation || !sameContext(snapshot, context)) stale();
      if (snapshot.phase !== 'RESTORED_LOCAL') throw new CushionError('core-rejected', 'ローカル復元を確認できませんでした。');
      const latest = await current(context, run);
      if (latest.phase !== 'RESTORED_LOCAL' || latest.restored !== snapshot.restored) stale();
      return {
        snapshot, outboundText, providerAnswer, restored: snapshot.restored,
        requestPreview: Object.freeze({ mode: 'demo', destination: exported.destination,
          method: 'SIMULATED', endpoint: null,
          body: Object.freeze({ text: outboundText }), externalRequestMade: false }),
      };
    } finally { roundTripRunning = false; }
  }

  async function sendApproved(_approvalId: string, _transport: ApprovedTransport): Promise<never> {
    // Native export_prepared is the standalone clipboard operation. Do not even
    // dispatch it from this disabled generic API seam: that could copy unexpectedly.
    // Only sendNative's explicit, approval-ID-only host capability may send an API request.
    throw new CushionError('production-unavailable', '任意の送信先を受け取るAPI接続は無効です。検証済みネイティブ接続のsendNativeを使用してください。コピーも送信もしていません。');
  }

  async function sendNative(approvalId: string): Promise<Snapshot> {
    if (!localOnlyHost || localOnlyHost.isNative !== true || typeof localOnlyHost.sendApproved !== 'function') {
      throw new CushionError('production-unavailable', '本番送信は検証済みのローカルホスト接続が必要です。ブラウザだけでは送信できません。');
    }
    if (roundTripRunning) throw new CushionError('busy', '承認済みの文章を処理しています。');
    const context = approvalContext; const run = generation;
    if (!context || context.approvalId !== approvalId) {
      throw new CushionError('review-required', 'この相談の最新の変換文を確認して承認してください。');
    }
    roundTripRunning = true;
    try {
      const before = await current(context, run);
      if (before.mode !== 'local' || before.phase !== 'APPROVED' || before.approvalId !== approvalId) stale();
      let result: Snapshot;
      try { result = await localOnlyHost.sendApproved(approvalId); }
      catch {
        // A raw host exception can include private payloads. Only a fresh,
        // same-consultation Core stop reason may be shown instead of that error.
        let stopped: Snapshot | undefined;
        try { stopped = await call({ op: 'snapshot' }); } catch { /* retain fixed fallback */ }
        if (stopped && (run !== generation || !sameContext(stopped, context))) stale();
        if (stopped?.phase === 'BLOCKED' && typeof stopped.blockedReason === 'string' && stopped.blockedReason.length > 0) {
          throw new CushionError('core-rejected', stopped.blockedReason);
        }
        throw new CushionError('core-rejected', 'ネイティブ側で送信・回答復元の成功を確認できませんでした。処理状態を確認してください。自動再送はしていません。');
      }
      if (run !== generation || !sameContext(result, context) || result.phase !== 'RESTORED_LOCAL') stale();
      const latest = await current(context, run);
      if (latest.phase !== 'RESTORED_LOCAL' || latest.restored !== result.restored) stale();
      return latest;
    } finally { roundTripRunning = false; }
  }

  return Object.freeze({ prepare, approve, runDemo, sendApproved, sendNative });
}
