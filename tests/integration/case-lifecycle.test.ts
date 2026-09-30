import { beforeAll, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { initSync, DemoCore } from '../../apps/desktop/src/wasm/cushion_core.js';
import type { CoreRequest, Snapshot } from '../../apps/desktop/src/types';

const host = vi.hoisted(() => ({ native: true, invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => host.native, invoke: (...args: unknown[]) => host.invoke(...args) }));
import { bridge, SAMPLE, startNewCase } from '../../apps/desktop/src/bridge';

beforeAll(() => initSync({ module: readFileSync(new URL('../../apps/desktop/src/wasm/cushion_core_bg.wasm', import.meta.url)) }));

describe('new-case host mode with actual Rust WASM', () => {
  for (const native of [true, false]) {
    it(`clears approval/jobs/tokens and establishes ${native ? 'native local' : 'browser demo'} before later edits`, async () => {
      host.native = native;
      const core = new DemoCore();
      host.invoke.mockImplementation(async (_command: string, args: { request: CoreRequest }) => JSON.parse(core.dispatch(JSON.stringify(args.request))));
      await bridge({ op: 'update', patch: { original: SAMPLE, mode: 'demo' } });
      const scanning = await bridge({ op: 'begin_scan' });
      const reviewed = await bridge({ op: 'finish_scan', jobId: scanning.jobId, fault: 'none' });
      const approved = await bridge({ op: 'approve', revision: reviewed.revision });
      const exported = await bridge({ op: 'demo_export', approvalId: approved.approvalId });
      const token = exported.findings.find(finding => finding.action === '置換')!.replacement;
      expect(exported.testCopy).toContain(token);

      // Queue a later edit immediately: it must see the new host mode already set.
      const reset = startNewCase();
      const edited = bridge({ op: 'update', patch: { original: SAMPLE } });
      const fresh = await reset;
      expect(fresh.caseId).not.toBe(exported.caseId);
      expect(fresh.mode).toBe(native ? 'local' : 'demo');
      expect(fresh.original).toBe('');
      expect(fresh.phase).toBe('DRAFT');
      expect(fresh.approvalId).toBeNull();
      expect(fresh.jobId).toBeNull();
      expect(fresh.transformed).toBe('');
      expect(fresh.findings).toEqual([]);
      expect(fresh.testCopy).toBeNull();
      expect(fresh.restored).toBe('');
      expect((await edited).mode).toBe(native ? 'local' : 'demo');
      await expect(bridge({ op: 'demo_export', approvalId: approved.approvalId })).rejects.toBeDefined();

      // A fresh demo round-trip proves the old token has no mapping in this case.
      await bridge({ op: 'update', patch: { mode: 'demo' } });
      const nextJob = await bridge({ op: 'begin_scan' });
      const nextReview = await bridge({ op: 'finish_scan', jobId: nextJob.jobId, fault: 'none' });
      const nextApproval = await bridge({ op: 'approve', revision: nextReview.revision });
      await bridge({ op: 'demo_export', approvalId: nextApproval.approvalId });
      const restored: Snapshot = await bridge({ op: 'restore', answer: token });
      expect(restored.restored).toBe(token);
      expect(restored.unknownTokens).toContain(token);

      await bridge({ op: 'update', patch: { purpose: 'summary' } });
      const obsoleteJob = await bridge({ op: 'begin_scan' });
      const blank = await startNewCase();
      expect(blank.jobId).toBeNull();
      expect(await bridge({ op: 'finish_scan', jobId: obsoleteJob.jobId, fault: 'none' })).toEqual(blank);
      expect((await bridge({ op: 'snapshot' })).transformed).toBe('');
    });
  }
});
