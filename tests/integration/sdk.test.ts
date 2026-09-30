import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { initSync, DemoCore } from '../../apps/desktop/src/wasm/cushion_core.js';
import { createCushionClient } from '../../packages/cushion-sdk/src/index';
import type { CoreRequest, PrepareInput, Snapshot } from '../../packages/cushion-sdk/src/contracts';
import supportFixtures from '../fixtures/workflows/support-record.json';

const fixtures = JSON.parse(readFileSync(new URL('../fixtures/synthetic/cases.json', import.meta.url), 'utf8'));
const sample: string = fixtures[0].original;
const input: PrepareInput = { original: sample, purpose: 'polish', destination: 'ChatGPT' };
beforeAll(() => initSync({ module: readFileSync(new URL('../../apps/desktop/src/wasm/cushion_core_bg.wasm', import.meta.url)) }));
afterEach(() => vi.restoreAllMocks());
const raw = (core: DemoCore, request: CoreRequest): Snapshot => JSON.parse(core.dispatch(JSON.stringify(request)));
type Hook = (request: CoreRequest, core: DemoCore) => void | Promise<void>;
function setup(hooks: { before?: Hook; after?: Hook } = {}) {
  const core = new DemoCore(); const calls: string[] = [];
  const dispatch = async (request: CoreRequest) => {
    calls.push(request.op);
    await hooks.before?.(request, core);
    const result = raw(core, request);
    await hooks.after?.(request, core);
    return result;
  };
  return { core, client: createCushionClient({ dispatch }), calls };
}
async function approved() {
  const h = setup(); const review = await h.client.prepare(input);
  const approvalId = await h.client.approve(review.revision);
  return { ...h, review, approvalId };
}

describe('Cushion SDK with actual Rust WASM', () => {
  it('round-trips the frozen support note with the Core-owned instruction and no network', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('must stay local'));
    const h = setup(); const record = supportFixtures[0];
    const s = await h.client.prepare({ ...input, purpose: 'support_record', original: record.original });
    expect(s.phase).toBe('REVIEW');
    expect(s.transformed).toContain('主語・否定・条件・出来事の順序を変えない');
    expect(s.transformed).toContain('最初は参加しなかった');
    expect(s.transformed).not.toContain('風待ち教室');
    const id = await h.client.approve(s.revision);
    const result = await h.client.runDemo(id);
    expect(result.restored).toContain('風待ち教室');
    expect(result.restored).toContain('葉村');
    expect(result.requestPreview.externalRequestMade).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
  });
  it('blocks medical/family support examples and refuses modified fixture export', async () => {
    for (const record of supportFixtures.slice(2)) {
      const h=setup();
      const s=await h.client.prepare({ ...input,purpose:'support_record',original:record.original });
      expect(s.phase).toBe('BLOCKED');
      await expect(h.client.approve(s.revision)).rejects.toThrow();
    }
    const h=setup();
    const s=await h.client.prepare({ ...input,purpose:'support_record',original:supportFixtures[0].original+'変更' });
    expect(s.phase).toBe('BLOCKED');
  });
  it('prepares a complete redacted review without exporting', async () => {
    const { client, core } = setup(); const s = await client.prepare(input);
    expect(s.phase).toBe('REVIEW'); expect(s.original).toBe(sample);
    for (const secret of fixtures[0].prohibited) expect(s.transformed).not.toContain(secret);
    expect(raw(core, { op: 'snapshot' }).testCopy).toBeNull();
  });
  it('completes one in-chat demo round trip with no network or provider callback', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('network must not run'));
    const h = await approved(); const result = await h.client.runDemo(h.approvalId);
    expect(result.snapshot.phase).toBe('RESTORED_LOCAL');
    expect(result.providerAnswer).toContain('固定デモ回答');
    expect(result.providerAnswer).not.toContain('小森');
    expect(result.restored).toContain('小森'); expect(result.restored).toContain('こもれび商店');
    expect(result.outboundText).not.toContain('こもれび商店');
    expect(result.requestPreview.externalRequestMade).toBe(false);
    expect(result.requestPreview.endpoint).toBeNull();
    expect(result.requestPreview.body).toEqual({ text: result.outboundText });
    expect(Object.keys(result.requestPreview.body)).toEqual(['text']);
    expect(fetch).not.toHaveBeenCalled();
    expect(h.calls).toContain('restore_if_current');
  });
  it('rejects demo execution before approval', async () => {
    const { client } = setup(); await client.prepare(input);
    await expect(client.runDemo('forged')).rejects.toMatchObject({ code: 'review-required' });
  });
  it('rejects approval without a prepared review', async () => {
    const { client } = setup(); await expect(client.approve(0)).rejects.toMatchObject({ code: 'review-required' });
  });
  it('rejects arbitrary original input through the actual Core', async () => {
    const { client } = setup(); const s = await client.prepare({ ...input, original: '架空ですがfixture外の文です。' });
    expect(s.phase).toBe('BLOCKED'); await expect(client.approve(s.revision)).rejects.toThrow();
  });
  it('does not scan the previous accepted source after an actual Core size rejection', async () => {
    const { client, core, calls } = setup();
    raw(core, { op: 'update', patch: { original: sample } });
    const blocked = await client.prepare({ ...input, original: 'X'.repeat(65537) });
    expect(blocked.phase).toBe('BLOCKED');
    expect(blocked.original).toBe(sample);
    expect(calls).not.toContain('begin_scan');
  });
  it('can shorten an original rejected above the raw JSON limit without acknowledging unrelated settings', async () => {
    const { client, core, calls } = setup();
    raw(core, { op: 'update', patch: { original: sample } });
    await expect(client.prepare({ ...input, original: 'X'.repeat(131073) })).rejects.toMatchObject({ code: 'core-rejected' });
    const blocked = raw(core, { op: 'snapshot' });
    expect(blocked.phase).toBe('BLOCKED');
    expect(blocked.original).toBe(sample);
    expect(calls).not.toContain('begin_scan');
    const repaired = await client.prepare(input);
    expect(repaired.phase).toBe('REVIEW');
    expect(repaired.original).toBe(sample);
  });
  it('blocks high-sensitivity synthetic input', async () => {
    const { client } = setup(); const s = await client.prepare({ ...input, original: fixtures.find((f: { id: string }) => f.id === 'medical').original });
    expect(s.phase).toBe('BLOCKED'); expect(s.blockedReason).toContain('診断');
  });
  for (const key of ['messages', 'system', 'history', 'tools', 'retrieval', 'apiKey']) {
    it(`rejects uninspected ${key} instead of passing it through`, async () => {
      const { client, calls } = setup();
      await expect(client.prepare({ ...input, [key]: '架空の秘密' } as PrepareInput)).rejects.toMatchObject({ code: 'invalid-input' });
      expect(calls).toEqual([]);
    });
  }
  it('makes zero IPC and transport calls from the disabled generic API seam in demo', async () => {
    const { client, calls, approvalId } = await approved(); const transport = vi.fn();
    const before = calls.length;
    await expect(client.sendApproved(approvalId, transport)).rejects.toMatchObject({ code: 'production-unavailable' });
    expect(calls.length).toBe(before); expect(transport).not.toHaveBeenCalled();
  });
  it('makes zero transport calls with the unready local model', async () => {
    const { client, core, calls } = setup(); raw(core, { op: 'update', patch: { mode: 'local' } });
    const s = await client.prepare(input); expect(s.phase).toBe('BLOCKED');
    const before = calls.length;
    const transport = vi.fn(); await expect(client.sendApproved('forged', transport)).rejects.toThrow();
    expect(calls.length).toBe(before); expect(transport).not.toHaveBeenCalled();
  });
  it('does not invoke a host export command even if it would return a snapshot', async () => {
    const core = new DemoCore();
    // Real WASM for every state operation. Only simulate an incompatible future
    // export response to prove the SDK does not leak the whole local snapshot.
    let exportCalled = false;
    const client = createCushionClient({ dispatch: async request => {
      if (request.op === 'export_prepared') exportCalled = true;
      return raw(core, request.op === 'export_prepared' ? { op: 'snapshot' } : request);
    } });
    const review = await client.prepare(input); const id = await client.approve(review.revision);
    const transport = vi.fn();
    await expect(client.sendApproved(id, transport)).rejects.toMatchObject({ code: 'production-unavailable' });
    expect(exportCalled).toBe(false); expect(transport).not.toHaveBeenCalled();
  });
  it('does not invoke a callback placed in unknown client options', async () => {
    const core = new DemoCore(); const transport = vi.fn();
    const options = { dispatch: async (request: CoreRequest) => raw(core, request), transport };
    const client = createCushionClient(options);
    const review = await client.prepare(input); const id = await client.approve(review.revision);
    await client.runDemo(id);
    expect(transport).not.toHaveBeenCalled();
  });
  it('does not send a demo approval through an injected native capability', async () => {
    const core = new DemoCore(); const sendApproved = vi.fn();
    const client = createCushionClient({ dispatch: async request => raw(core, request),
      localOnlyHost: { isNative: true, sendApproved } });
    const review = await client.prepare(input); const id = await client.approve(review.revision);
    await expect(client.sendNative(id)).rejects.toThrow();
    expect(sendApproved).not.toHaveBeenCalled();
  });
  for (const patch of [
    { original: sample + '。' }, { transformed: '架空の変更' }, { purpose: 'summary' },
    { destination: 'Claude' }, { dictionaryText: '新しい架空辞書' },
    { policyVersion: 'new' }, { modelConfig: 'new' },
  ]) {
    it(`does not reuse approval after ${Object.keys(patch)[0]} changes`, async () => {
      const h = await approved(); raw(h.core, { op: 'update', patch });
      await expect(h.client.runDemo(h.approvalId)).rejects.toMatchObject({ code: 'stale-result' });
      expect(raw(h.core, { op: 'snapshot' }).testCopy).toBeNull();
    });
  }
  it('rejects a delayed prepared result after another edit', async () => {
    const { client, core } = setup({ after: (request, c) => {
      if (request.op === 'finish_scan') raw(c, { op: 'update', patch: { original: sample + '架空追記' } });
    } });
    await expect(client.prepare(input)).rejects.toMatchObject({ code: 'stale-result' });
    expect(raw(core, { op: 'snapshot' }).phase).toBe('DRAFT');
  });
  it('rejects an edit between update and begin_scan', async () => {
    const { client, calls } = setup({ after: (request, c) => {
      if (request.op === 'update') raw(c, { op: 'update', patch: { destination: 'Claude' } });
    } });
    await expect(client.prepare(input)).rejects.toMatchObject({ code: 'stale-result' });
    expect(calls).not.toContain('begin_scan');
  });
  it('rejects a case switch after export before answer restoration', async () => {
    const { client, core } = setup({ after: (request, c) => {
      if (request.op === 'demo_export') raw(c, { op: 'new_case' });
    } });
    const r = await client.prepare(input); const id = await client.approve(r.revision);
    await expect(client.runDemo(id)).rejects.toMatchObject({ code: 'stale-result' });
    expect(raw(core, { op: 'snapshot' }).restored).toBe('');
  });
  it('atomically rejects an edit inside the final restoration call', async () => {
    const { client, core } = setup({ before: (request, c) => {
      if (request.op === 'restore_if_current') raw(c, { op: 'update', patch: { purpose: 'summary' } });
    } });
    const r = await client.prepare(input); const id = await client.approve(r.revision);
    await expect(client.runDemo(id)).rejects.toMatchObject({ code: 'core-rejected' });
    const s = raw(core, { op: 'snapshot' }); expect(s.phase).toBe('DRAFT'); expect(s.restored).toBe('');
  });
  it('atomically rejects cancellation immediately before restoration', async () => {
    const { client, core } = setup({ before: (request, c) => {
      if (request.op === 'restore_if_current') raw(c, { op: 'cancel' });
    } });
    const r = await client.prepare(input); const id = await client.approve(r.revision);
    await expect(client.runDemo(id)).rejects.toThrow();
    expect(raw(core, { op: 'snapshot' }).restored).toBe('');
  });
  it('keeps unknown and foreign tokens and markup as plain text through conditional restore', async () => {
    const a = await approved(); const b = await approved();
    const ae = raw(a.core, { op: 'demo_export', approvalId: a.approvalId });
    const be = raw(b.core, { op: 'demo_export', approvalId: b.approvalId });
    const answer = `${ae.findings[0].replacement} ${be.findings[0].replacement} [[UNKNOWN]] <img src="https://example.invalid/x"><script>anything()</script>`;
    const s = raw(a.core, { op: 'restore_if_current', answer, caseId: ae.caseId, revision: ae.revision });
    expect(s.restored).toContain('こもれび商店'); expect(s.restored).toContain(be.findings[0].replacement);
    expect(s.restored).toContain('[[UNKNOWN]]'); expect(s.restored).toContain('<img src=');
    expect(s.unknownTokens).toHaveLength(2);
  });
  it('rejects double submission without a second export', async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const h = setup({ before: async request => { if (request.op === 'demo_export') await gate; } });
    const review = await h.client.prepare(input); const id = await h.client.approve(review.revision);
    const first = h.client.runDemo(id);
    await expect(h.client.runDemo(id)).rejects.toMatchObject({ code: 'busy' });
    release(); await first;
    expect(h.calls.filter(op => op === 'demo_export')).toHaveLength(1);
  });
  it('does not echo host exceptions that might contain document text', async () => {
    const client = createCushionClient({ dispatch: async () => { throw new Error('SYNTHETIC-CANARY-SECRET'); } });
    try { await client.prepare(input); throw new Error('must reject'); }
    catch (error) { expect(String(error)).not.toContain('SYNTHETIC-CANARY-SECRET'); }
  });
});
