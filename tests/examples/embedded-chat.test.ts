import { beforeAll, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { initSync, DemoCore } from '../../apps/desktop/src/wasm/cushion_core.js';
import { connectNativeChat, connectOfflineDemo, nativePorts } from '../../examples/embedded-chat';
import type { LocalInvoke } from '../../examples/embedded-chat';
import type { CoreRequest, Snapshot } from '../../packages/cushion-sdk/src/contracts';

const fixtures = JSON.parse(readFileSync(new URL('../fixtures/synthetic/cases.json', import.meta.url), 'utf8'));
const sample = fixtures[0];
beforeAll(() => initSync({ module: readFileSync(new URL('../../apps/desktop/src/wasm/cushion_core_bg.wasm', import.meta.url)) }));
const input = { original: sample.original as string, purpose: 'polish' as const, destination: 'ChatGPT' as const };

function local() {
  const core = new DemoCore();
  const raw = (request: CoreRequest): Snapshot => JSON.parse(core.dispatch(JSON.stringify(request)));
  const dispatch = async (request: CoreRequest) => raw(request);
  return { raw, dispatch, client: connectOfflineDemo(dispatch) };
}

describe('組み込み例: 実Rust WASM・架空データ・外部通信なし', () => {
  it('① 検査 → 比較 → 明示承認 → 固定デモ回答 → 同じ相談で復元', async () => {
    const network = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('Network forbidden in this example'));
    try {
      const h = local();
      const review = await h.client.prepare(input);
      expect(review.phase).toBe('REVIEW');
      expect(review.original).toBe(input.original);
      expect(h.raw({ op: 'snapshot' }).testCopy).toBeNull();
      for (const secret of sample.prohibited) expect(review.transformed).not.toContain(secret);

      // A real UI calls approve only from the user's confirmation action.
      const approvalId = await h.client.approve(review.revision);
      const result = await h.client.runDemo(approvalId);
      expect(result.snapshot.phase).toBe('RESTORED_LOCAL');
      expect(result.requestPreview.externalRequestMade).toBe(false);
      expect(result.requestPreview.endpoint).toBeNull();
      expect(result.outboundText).toBe(review.transformed);
      expect(result.providerAnswer).toContain('固定デモ回答');
      for (const finding of review.findings) {
        expect(result.providerAnswer).toContain(finding.replacement);
        expect(result.restored).toContain(finding.original);
      }
      expect(network).not.toHaveBeenCalled();
    } finally { network.mockRestore(); }
  });

  it('② 未承認では持ち出せず、承認後の編集でも古い承認を使えない', async () => {
    const h = local();
    await expect(h.client.runDemo('unapproved-example')).rejects.toThrow();
    const review = await h.client.prepare(input);
    const id = await h.client.approve(review.revision);
    h.raw({ op: 'update', patch: { original: input.original + '架空の追記。' } });
    await expect(h.client.runDemo(id)).rejects.toThrow();
    expect(h.raw({ op: 'snapshot' }).testCopy).toBeNull();
  });

  it('③ 処理中に編集したら、遅い検査結果を採用しない', () => {
    const h = local();
    h.raw({ op: 'update', patch: input });
    const pending = h.raw({ op: 'begin_scan' });
    const edited = h.raw({ op: 'update', patch: { original: input.original + '架空の追記。' } });
    try { h.raw({ op: 'finish_scan', jobId: pending.jobId, fault: 'none' }); } catch { /* Core may reject the old command. */ }
    const current = h.raw({ op: 'snapshot' });
    expect(current.revision).toBe(edited.revision);
    expect(current.original).toBe(edited.original);
    expect(current.transformed).toBe('');
    expect(current.approvalId).toBeNull();
  });

  it('④ 検査が異常終了したら、原文を保持して持ち出しを止める', () => {
    const h = local();
    h.raw({ op: 'update', patch: input });
    const pending = h.raw({ op: 'begin_scan' });
    const stopped = h.raw({ op: 'finish_scan', jobId: pending.jobId, fault: 'invalid_json' });
    expect(stopped.phase).toBe('BLOCKED');
    expect(stopped.original).toBe(input.original);
    expect(stopped.testCopy).toBeNull();
    expect(stopped.approvalId).toBeNull();
  });

  it('⑤ native送信コマンドへ渡すのは承認IDだけ', async () => {
    const h = local();
    const invoke = vi.fn(async () => h.raw({ op: 'snapshot' }));
    const ports = nativePorts(invoke as LocalInvoke);
    await ports.localOnlyHost.sendApproved('example-approval-id');
    expect(invoke).toHaveBeenCalledExactlyOnceWith('api_send', { approvalId: 'example-approval-id' });
    // This is a contract test, not native IPC or a real provider request.
  });

  it('⑥ demoをnativeと呼び変えても実送信へ進めない', async () => {
    const h = local();
    const invoke = vi.fn(async (command: string, args: { request?: CoreRequest }) => {
      if (command !== 'dispatch' || !args.request) throw new Error('Unexpected native operation');
      return h.raw(args.request);
    });
    const client = connectNativeChat(invoke as LocalInvoke);
    const review = await client.prepare(input);
    const id = await client.approve(review.revision);
    await expect(client.sendNative(id)).rejects.toThrow();
    expect(invoke.mock.calls.some(([command]) => command === 'api_send')).toBe(false);
  });
});
