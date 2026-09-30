import { useEffect, useRef, useState, type ReactNode } from 'react';
import { bridge, deleteCase, getVaultStatus, isNative, listVaultEntries, loadCase, lockVault, modelStatus, reloadModel, saveCase, startNewCase, unlockVault } from './bridge';
import type { ModelStatus, Snapshot, VaultEntry, VaultStatus } from './types';
import { decodeTextFile, MAX_TEXT_FILE_BYTES } from './textImport';
import './styles.css';

const SAMPLE = 'こもれび商店の小森さんへ、発送日が9月20日から9月23日に変わるお知らせを書いてください。注文番号はKMR-4821です。';
const PURPOSES = [
  ['polish', '文章を整える'],
  ['summary', '要点を整理する'],
  ['perspectives', '一般的な観点を出す'],
  ['support_record', '支援記録の文章を整える'],
] as const;
type Request = Parameters<typeof bridge>[0];
type Fault = 'none' | 'invalid_json' | 'stopped' | 'incomplete';
const draftPatch = (draft: Snapshot) => ({
  original: draft.original, purpose: draft.purpose, destination: draft.destination,
  mode: draft.mode, dictionaryText: draft.dictionaryText, policyVersion: draft.policyVersion,
  ...(draft.mode === 'demo' ? { modelConfig: draft.modelConfig } : {}),
});
const matchesDraft = (next: Snapshot, patch: ReturnType<typeof draftPatch>) => Object.entries(patch).every(([key, value]) => next[key as keyof Snapshot] === value);
function requireAcceptedDraft(next: Snapshot | null, patch: ReturnType<typeof draftPatch>): asserts next is Snapshot {
  // Core size/policy rejection can return Ok(BLOCKED) while retaining the old
  // source. A fulfilled IPC promise does not mean this visible draft was accepted.
  if (!next || next.phase === 'BLOCKED' || !matchesDraft(next, patch)) {
    throw new Error(next?.blockedReason || '入力や設定の全文を受理できませんでした。画面の内容を短くするなど修正して、もう一度確認してください。');
  }
}
async function synchronizeDraft(patch: ReturnType<typeof draftPatch>): Promise<Snapshot> {
  // A valid update explicitly acknowledges corrected fields in Core, including
  // shortening back to the previous accepted value. Cancel cannot clear rejection.
  const next = await bridge({ op: 'update', patch });
  requireAcceptedDraft(next, patch);
  return next;
}

function LeafMark({ className = '' }: { className?: string }) {
  return <svg className={className} width="38" height="38" viewBox="0 0 40 40" fill="none" aria-hidden="true"><path d="M30 8C16 6 7 12 10 23c2 8 15 10 21-1 2-4 1-10-1-14Z" stroke="currentColor" strokeWidth="1.3"/><path d="m9 33 17-20M17 24l-1-9M21 20l8 1" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round"/></svg>;
}

function Arrow({ back = false }: { back?: boolean }) {
  return <span aria-hidden="true" className="button-arrow">{back ? '←' : '→'}</span>;
}

function ActionLabel({ children, action }: { children: ReactNode; action: string }) {
  return <span className={`change-label change-${action === '停止' ? 'stop' : 'replace'}`}>{action}<span className="sr-only">: </span>{children}</span>;
}

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [settings, setSettings] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const [motionOff, setMotionOff] = useState(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const [companion, setCompanion] = useState(true);
  const [answer, setAnswer] = useState('');
  const [answerOpen, setAnswerOpen] = useState(false);
  const [editingAnswer, setEditingAnswer] = useState(false);
  const [fault, setFault] = useState<Fault>('none');
  const [holdScan, setHoldScan] = useState(false);
  const [heldJob, setHeldJob] = useState<string | null>(null);
  const [notice, setNotice] = useState('');
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [modelError, setModelError] = useState('');
  const [vault, setVault] = useState<VaultStatus | null>(null);
  const [vaultEntries, setVaultEntries] = useState<VaultEntry[]>([]);
  const [selectedSave, setSelectedSave] = useState('');
  const [vaultBusy, setVaultBusy] = useState(false);
  const [vaultNotice, setVaultNotice] = useState('');
  const [vaultError, setVaultError] = useState('');
  const [deleteConfirmed, setDeleteConfirmed] = useState(false);
  const native = isNative();
  const epoch = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const busyRef = useRef(false);
  const originalRef = useRef<HTMLTextAreaElement>(null);
  const transformedRef = useRef<HTMLTextAreaElement>(null);
  const answerRef = useRef<HTMLTextAreaElement>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const settingsButton = useRef<HTMLButtonElement>(null);
  const contentHeading = useRef<HTMLHeadingElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  // All Core mutations are ordered. The epoch also rejects responses superseded
  // by an edit before IPC completes; Core independently checks revision/job IDs.
  const enqueue = (request: Request, guard?: () => boolean): Promise<Snapshot | null> => {
    const pending = queue.current.catch(() => undefined).then(() => guard && !guard() ? null : bridge(request));
    queue.current = pending;
    return pending;
  };
  const enqueueNative = <T,>(operation: () => Promise<T>): Promise<T> => {
    // Include native vault mutations in the same UI queue as edits, rather than
    // relying on the order in which separate IPC requests acquire a Core mutex.
    const pending = queue.current.catch(() => undefined).then(() => operation());
    queue.current = pending;
    return pending;
  };

  useEffect(() => {
    const current = epoch.current;
    enqueue({ op: 'snapshot' }).then(async next => {
      if (native && next && next.mode !== 'local') next = await enqueue({ op: 'update', patch: { mode: 'local' } });
      if (next && epoch.current === current) setSnapshot(next);
    })
      .catch(() => setError('このPC内の処理を開始できませんでした。起動手順を確認して、アプリを開き直してください。'));
    refreshModel();
    if (native) getVaultStatus().then(async status => {
      setVault(status);
      if (status.unlocked) setVaultEntries(await enqueueNative(listVaultEntries));
    }).catch(() => setVaultError('暗号化保存の状態を確認できませんでした。保存は利用できません。'));
  }, []);

  // Native analysis finishes in the host worker. UI never supplies model output
  // or calls finish_scan in local mode. A changed edit/job cancels adoption.
  useEffect(() => {
    if (!native || snapshot?.mode !== 'local' || snapshot.phase !== 'SCANNING' || !snapshot.jobId) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const current = epoch.current;
    const { caseId, revision, jobId } = snapshot;
    const poll = async () => {
      try {
        const next = await enqueue({ op: 'snapshot' }, () => active && epoch.current === current);
        if (!active || epoch.current !== current || !next) return;
        if (next.caseId !== caseId || next.revision !== revision) return;
        if (next.phase === 'SCANNING') {
          if (next.jobId === jobId) timer = setTimeout(poll, 500);
          return;
        }
        setSnapshot(next);
        refreshModel();
        if (next.phase === 'REVIEW' || next.phase === 'BLOCKED') requestAnimationFrame(() => contentHeading.current?.focus());
      } catch {
        if (!active || epoch.current !== current) return;
        setError('ローカル検査の状態を確認できませんでした。コピーを止めています。入力は残っています。');
        try {
          const canceled = await enqueue({ op: 'cancel' });
          if (active && current === epoch.current && canceled) setSnapshot(canceled);
        } catch { /* The existing SCANNING state keeps export unavailable. */ }
      }
    };
    timer = setTimeout(poll, 500);
    return () => { active = false; if (timer) clearTimeout(timer); };
  }, [native, snapshot?.mode, snapshot?.phase, snapshot?.jobId, snapshot?.caseId, snapshot?.revision]);

  async function refreshModel() {
    try { setModel(await modelStatus()); setModelError(''); }
    catch { setModelError('モデルの状態を確認できませんでした。実推論を準備済みとは扱いません。'); setModel(null); }
  }

  useEffect(() => {
    // Worker shutdown may finish after an edit/cancel already returned to DRAFT.
    // Follow its real status independently of the inspection-screen lifecycle.
    if (!native || (model?.state !== 'verifying' && model?.state !== 'busy')) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await modelStatus();
        if (!active) return;
        setModel(next); setModelError('');
        if (next.state === 'verifying' || next.state === 'busy') timer = setTimeout(poll, 1000);
      } catch { if (active) setModelError('モデルの状態を確認できませんでした。'); }
    };
    timer = setTimeout(poll, 1000);
    return () => { active = false; clearTimeout(timer); };
  }, [native, model?.state]);

  const prepareModel = async () => {
    if (!native || busyRef.current) return;
    const current = ++epoch.current;
    setConfirmed(false); busyRef.current = true; setBusy(true); setModelError('');
    try {
      const canceled = await enqueue({ op: 'cancel' });
      if (current !== epoch.current) return;
      if (canceled) setSnapshot({ ...canceled, original: snapshot?.original ?? canceled.original });
      await reloadModel();
      await refreshModel();
      setNotice('モデルを再準備しています。前の承認は引き継ぎません。');
    } catch { setModelError('モデルを再準備できませんでした。処理中の場合は、完了してからお試しください。'); }
    finally { busyRef.current = false; setBusy(false); }
  };

  useEffect(() => {
    const dialog = dialogRef.current;
    if (settings && dialog && !dialog.open) dialog.showModal();
    if (!settings && dialog?.open) dialog.close();
  }, [settings]);

  const reportError = (err: unknown) => {
    setError(err instanceof Error ? err.message : typeof err === 'string' ? err : '処理が完了しませんでした。入力はそのまま残っています。');
  };

  const edit = (patch: Record<string, unknown>) => {
    const current = ++epoch.current;
    setConfirmed(false);
    setError('');
    setNotice('内容を変更しました。コピーする前に、もう一度確認・変換してください。');
    setAnswerOpen(false);
    setSnapshot(old => old ? { ...old, ...patch, phase: 'DRAFT', approvalId: null, jobId: null, transformed: '', findings: [], residual: [], lost: [], blockedReason: null, restored: '', testCopy: null } as Snapshot : old);
    // Preserve a draft rejected by a previous size/format check when a setting
    // changes: do not replace that visible draft with the Core's older source.
    if (!snapshot) return;
    const requestPatch = draftPatch({ ...snapshot, ...patch } as Snapshot);
    enqueueNative(() => synchronizeDraft(requestPatch)).then(next => {
      if (current !== epoch.current) return;
      setSnapshot(next);
    }).catch(err => { if (current === epoch.current) reportError(err); });
  };

  const runAction = async (request: Request, then?: (next: Snapshot) => void) => {
    if (busyRef.current) return;
    const current = ++epoch.current;
    busyRef.current = true; setBusy(true); setError('');
    try {
      const next = await enqueue(request);
      if (next && epoch.current === current) { setSnapshot(next); then?.(next); }
    } catch (err) { if (epoch.current === current) reportError(err); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const finishScan = async (jobId: string, current: number) => {
    if (snapshot?.mode !== 'demo') return;
    try {
      const next = await enqueue({ op: 'finish_scan', jobId, fault });
      if (next && epoch.current === current) {
        setSnapshot(next); setHeldJob(null);
        if (next.phase === 'REVIEW' || next.phase === 'BLOCKED') requestAnimationFrame(() => contentHeading.current?.focus());
      }
    } catch (err) { if (epoch.current === current) reportError(err); }
  };

  const scan = async () => {
    if (busyRef.current || !snapshot) return;
    const current = ++epoch.current;
    busyRef.current = true; setBusy(true); setError(''); setConfirmed(false); setNotice('');
    try {
      // A previous rejected update may have left a newer visible draft than the
      // Core contains. Re-send it before starting, so old source cannot be scanned.
      const patch = draftPatch(snapshot);
      const synchronized = await enqueueNative(() => epoch.current === current ? synchronizeDraft(patch) : Promise.resolve(null));
      if (epoch.current !== current) return;
      requireAcceptedDraft(synchronized, patch);
      const next = await enqueue({ op: 'begin_scan' }, () => epoch.current === current);
      if (!next || current !== epoch.current) return;
      setSnapshot(next);
      if (next.phase === 'SCANNING' && next.jobId) {
        if (next.mode === 'local') { setHeldJob(null); refreshModel(); }
        else if (holdScan) setHeldJob(next.jobId);
        else await finishScan(next.jobId, current);
      }
    } catch (err) { if (current === epoch.current) reportError(err); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const copy = async () => {
    if (!snapshot || !confirmed || busyRef.current) return;
    const current = ++epoch.current;
    busyRef.current = true; setBusy(true); setError('');
    try {
      const approved = await enqueue({ op: 'approve', revision: snapshot.revision }, () => epoch.current === current);
      if (!approved || epoch.current !== current) return;
      if (!approved.approvalId) { setSnapshot(approved); setConfirmed(false); return; }
      const exported = await enqueue({ op: snapshot.mode === 'demo' ? 'demo_export' : 'export_prepared', approvalId: approved.approvalId }, () => epoch.current === current);
      if (exported && epoch.current === current) {
        setSnapshot(exported);
        setConfirmed(false); setNotice('');
        requestAnimationFrame(() => contentHeading.current?.focus());
      }
    } catch (err) { if (current === epoch.current) { setConfirmed(false); reportError(err); } }
    finally { busyRef.current = false; setBusy(false); }
  };

  const loadSample = () => {
    edit({ original: SAMPLE });
    setNotice('架空のお知らせ文を置きました。実在の情報は含まれていません。');
    requestAnimationFrame(() => originalRef.current?.focus());
  };

  const importText = async (file: File | undefined) => {
    if (!file || !native) return;
    if (busyRef.current || snapshot?.phase === 'SCANNING') return;
    const current = ++epoch.current;
    setConfirmed(false); setError(''); setBusy(true); busyRef.current = true;
    try {
      if (!/\.txt$/i.test(file.name)) throw new Error('UTF-8のTXTファイルを選んでください。PDF・Office・画像は読み込みません。');
      if (file.size > MAX_TEXT_FILE_BYTES) throw new Error('ファイルが64 KiBを超えています。短いTXTファイルに分けてください。');
      const text = decodeTextFile(file.name, new Uint8Array(await file.arrayBuffer()));
      if (epoch.current !== current) return;
      edit({ original: text });
      setNotice('TXTをこのPC内で読み込みました。まだ確認・変換していません。');
      requestAnimationFrame(() => originalRef.current?.focus());
    } catch (reason) { if (epoch.current === current) reportError(reason); }
    finally { setBusy(false); busyRef.current = false; if (fileRef.current) fileRef.current.value = ''; }
  };

  const vaultAction = async (action: 'unlock' | 'save' | 'load' | 'lock' | 'delete') => {
    if (!native || vaultBusy) return;
    setVaultBusy(true); setVaultError(''); setVaultNotice('');
    try {
      if (action === 'unlock') {
        setVault(await enqueueNative(unlockVault));
        setVaultEntries(await enqueueNative(listVaultEntries));
        setVaultNotice('暗号化保存を開きました。保存は明示操作のときだけ行います。');
      } else if (action === 'save') {
        if (!snapshot) throw new Error('保存する相談を確認できませんでした。');
        const draft = draftPatch(snapshot);
        // The last edit can fail after Save was clicked. Do not silently save
        // the Core's older text; synchronize the visible draft in this turn.
        const entry = await enqueueNative(async () => {
          await synchronizeDraft(draft);
          return saveCase();
        });
        setVaultEntries(await enqueueNative(listVaultEntries)); setSelectedSave(entry.id);
        setVaultNotice('保存時点の相談を暗号化して保存しました。');
      } else if (action === 'load' && selectedSave) {
        const current = ++epoch.current;
        setConfirmed(false);
        const restored = await enqueueNative(() => loadCase(selectedSave));
        if (current === epoch.current) {
          setSnapshot(restored); setAnswer(''); setAnswerOpen(false); setEditingAnswer(false); setHeldJob(null);
          setNotice('保存した相談を開きました。承認は引き継がず、もう一度確認・変換します。');
          setVaultNotice('保存した相談を開きました。持ち出す前に再検査してください。');
        }
      } else if (action === 'lock') {
        setVault(await enqueueNative(lockVault)); setVaultEntries([]); setSelectedSave(''); setDeleteConfirmed(false);
        setVaultNotice('暗号化保存を閉じました。画面に開いている相談は、アプリを閉じるまでメモリに残ります。');
      } else if (action === 'delete' && selectedSave && deleteConfirmed) {
        const deleted = await enqueueNative(() => deleteCase(selectedSave));
        if (!deleted) throw new Error('選択した保存が見つかりませんでした。');
        setVaultEntries(await enqueueNative(listVaultEntries)); setSelectedSave(''); setDeleteConfirmed(false);
        setVaultNotice('選択した保存を削除しました。画面の相談は変更していません。');
      }
    } catch (reason) {
      setVaultError(errorTextForVault(reason));
      try { setVault(await getVaultStatus()); } catch { setVault(null); }
    } finally { setVaultBusy(false); }
  };
  const errorTextForVault = (reason: unknown) => typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : '暗号化保存の操作を完了できませんでした。平文では保存しません。';

  const openAnswer = () => {
    setAnswerOpen(true);
    requestAnimationFrame(() => answerRef.current?.focus());
  };

  const loadAnswerSample = () => {
    if (!snapshot) return;
    // Native candidates can include dates or honorifics, in any order. Echo the
    // exported text rather than guessing token roles or adding another suffix.
    setAnswer(`【架空の回答例・持ち出した文を使う復元確認】\n\n${snapshot.transformed}\n\nこれは復元操作を確かめるための合成文です。外部AIの回答ではありません。`);
    requestAnimationFrame(() => answerRef.current?.focus());
  };

  const closeSettings = () => { setSettings(false); requestAnimationFrame(() => settingsButton.current?.focus()); };
  const restart = async () => {
    if (busyRef.current) return;
    const current = ++epoch.current;
    busyRef.current = true; setBusy(true); setConfirmed(false); setHeldJob(null); setError('');
    try {
      const fresh = await enqueueNative(startNewCase);
      if (epoch.current !== current) return;
      setSnapshot(fresh); setAnswer(''); setAnswerOpen(false); setEditingAnswer(false);
      setNotice('新しい相談です。前の相談の登録トークンは引き継ぎません。');
    } catch (reason) { if (epoch.current === current) reportError(reason); }
    finally { busyRef.current = false; setBusy(false); }
  };
  const isReview = snapshot && ['REVIEW', 'APPROVED'].includes(snapshot.phase);
  const isExported = snapshot && ['EXPORTED', 'RESTORED_LOCAL'].includes(snapshot.phase);
  const isBlocked = snapshot?.phase === 'BLOCKED';
  const isScanning = snapshot?.phase === 'SCANNING';
  const isLocal = snapshot?.mode === 'local';
  const currentStep = isExported ? 2 : isReview ? 1 : 0;
  const stageTitle = isReview ? '持ち出す前に、ひと目。' : isExported ? answerOpen || snapshot.phase === 'RESTORED_LOCAL' ? '返ってきたことば。' : '手紙は、まだここに。' : isBlocked ? 'この文章は、ここで止めています。' : 'ことばを、ここに置いてください。';

  return <div className={`letter-room ${quiet ? 'quiet' : ''} ${motionOff ? 'motion-off' : ''} ${isBlocked || error || modelError || vaultError || model?.state === 'error' ? 'is-stopped motion-off' : ''}`}>
    <div className="room-scenery" aria-hidden="true"><div className="forest-layer"/><div className="room-middle-layer"/><div className="window-light"/><div className="room-vignette"/><div className="desk-layer"/><img className="desk-envelope" src="/art/envelope-closed.svg" alt=""/><div className="cup"><i/></div><div className="thread-spool"/></div>
    <a className="skip-link" href="#letter-main">本文と操作へ移動</a>
    <header className="room-header">
      <div className="wordmark"><LeafMark/><div><span>クッションAI</span><small>森の小さな手紙室</small></div></div>
      <div className="header-right"><span className="consultation-label">相談1<span aria-hidden="true"> / </span>このPC内</span><button ref={settingsButton} className="settings-button" onClick={() => setSettings(true)} aria-haspopup="dialog"><svg width="19" height="19" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="M3 5h14M3 10h14M3 15h14" stroke="currentColor"/><path d="M7 3v4m6 1v4m-7 1v4" stroke="currentColor" strokeWidth="3"/></svg>設定</button></div>
    </header>
    <div className="room-meta"><span className="demo-label"><i aria-hidden="true"/>{isLocal ? '実推論の試作・架空サンプルの持ち出しのみ' : 'デモ・実データ不可'}</span><span>業務利用未承認</span></div>
    <main id="letter-main" className={`letter-main ${isReview ? 'spread-main' : ''}`}>
      <nav className="journey" aria-label="相談の流れ"><ol>{['ことばを置く', '持ち出す文を確かめる', '回答を持ち帰る'].map((label, index) => <li key={label} aria-current={currentStep === index ? 'step' : undefined}><span className="journey-dot" aria-hidden="true"/>{label}</li>)}</ol></nav>
      <div className="scene-heading"><span className="eyebrow">A LITTLE ROOM FOR YOUR WORDS</span><h1 ref={contentHeading} tabIndex={-1}>{stageTitle}</h1><p>{isReview ? '隠したところも、外へ残ることも。両方を確かめてください。' : isExported ? '名前を戻すのは、この部屋の中だけで。' : '原文を入力。このアプリからは外部へ送信しません。'}</p></div>
      {!snapshot ? <section className="paper loading-paper" role="status">{error || 'このPC内の手紙室を開いています。'}</section> : <>
        <div className="destination-strip"><span className="local-note"><svg width="15" height="16" viewBox="0 0 15 16" fill="none" aria-hidden="true"><rect x="2" y="7" width="11" height="8" rx="1" stroke="currentColor"/><path d="M4.5 7V4a3 3 0 0 1 6 0v3" stroke="currentColor"/></svg>{native ? '自動保存なし・このPC内' : 'この相談はメモリ内のみ'}</span><label>送信予定先 <select aria-label="送信予定先" value={snapshot.destination} onChange={event => edit({ destination: event.target.value })}><option>ChatGPT</option><option>Claude</option><option>Gemini</option></select></label></div>
        {error && <div className="stop-note" role="alert"><strong>操作は完了していません。</strong><p>{error}</p><p>入力は残っています。変換文の持ち出しは停止しています。</p></div>}
        {(isReview) ? <>
          <div className="paper-spread">
            <section className="paper original-paper"><div className="paper-heading"><span className="paper-kicker">この部屋に残るもの</span><h2>原文・このPC内</h2></div><textarea ref={originalRef} aria-label="原文・このPC内" value={snapshot.original} onChange={event => edit({ original: event.target.value })} spellCheck={false} autoComplete="off" className="review-text original-text"/><p className="paper-footnote">編集すると、今回の確認と承認は無効になります。</p></section>
            <section className="paper prepared-paper"><div className="paper-heading"><span className="paper-kicker">あなたが確かめてから</span><h2>外へ渡す文</h2><span className="paper-stamp">未コピー</span></div><textarea ref={transformedRef} aria-label="外へ渡す文" value={snapshot.transformed} readOnly spellCheck={false} className="review-text prepared-text"/><p className="paper-footnote">{isLocal ? '表示している文字列だけを、OSクリップボードへコピーします。' : '表示している文字列が、テスト用コピーの全文です。'}</p></section>
          </div>
          <section className="review-notes" aria-label="変えたことと残ること"><div className="changes"><h3>ことばの置きかえ</h3><ul>{snapshot.findings.map((finding, index) => <li key={`${finding.start}-${index}`}><button onClick={() => {
            originalRef.current?.focus();
            const start = Array.from(snapshot.original).slice(0, finding.start).join('').length;
            const end = Array.from(snapshot.original).slice(0, finding.end).join('').length;
            originalRef.current?.setSelectionRange(start, end);
          }} className="finding-button"><ActionLabel action={finding.action}><span className="finding-original">{finding.original}</span></ActionLabel><span>{finding.reason}</span></button></li>)}</ul></div><div className="meaning-notes"><div><h3><span aria-hidden="true">↗</span> 外へ残る情報</h3><ul>{snapshot.residual.map((item, index) => <li key={index}>{item}</li>)}</ul></div><div><h3><span aria-hidden="true">↘</span> 失われる意味</h3><ul>{snapshot.lost.length ? snapshot.lost.map((item, index) => <li key={index}>{item}</li>) : <li>追加の意味損失は報告されていません。原文と読み比べて確認してください。</li>}</ul></div></div></section>
          <section className="approval-area"><label className="confirmation"><input type="checkbox" checked={confirmed} onChange={event => setConfirmed(event.target.checked)}/><span>変換文の全文、外へ残る情報、失われる意味を確認しました。</span></label><div className="copy-row"><span className="copy-explanation">{isLocal ? '変換文をコピー' : 'テスト用コピー'}<br/><small>{isLocal ? 'OSクリップボードへ入ります。AIへの送信は手動です。' : 'OSクリップボードへは書き込みません。'}</small></span><button className="primary-button" disabled={!confirmed || busy || Boolean(error)} onClick={copy}>確認して変換文をコピー<Arrow/></button></div><p className="caution-line">{isLocal ? '持ち出せるのは、アプリ内で用意した架空サンプルだけです。ローカルモデルによる検査は、業務で使える精度の認証ではありません。全文をご自身でも確かめてください。' : 'デモは辞書と架空の検査結果を使います。ローカルLLMによる検査ではありません。'}</p></section>
        </> : isExported ? <section className="paper return-paper">
          {!answerOpen && snapshot.phase !== 'RESTORED_LOCAL' ? <><div className="letter-illustration" aria-hidden="true"><img src="/art/envelope-closed.svg" alt=""/><span>LOCAL ONLY</span></div><div className="copy-success" role="status"><h2>{isLocal ? '変換文をコピーしました。' : 'テスト用に変換文をコピーしました。'}</h2><p>まだAIには送信していません。</p></div><p className="gentle-note">{isLocal ? <>変換文だけをOSクリップボードに入れました。<br className="wide-break"/>ご自身で{snapshot.destination}へ貼り付け、確認して送信してください。</> : <>このデモでは、OSクリップボードに書き込まず、<br className="wide-break"/>このアプリ内でコピーと復元の流れを確かめます。</>}</p><details className="copied-preview"><summary>{isLocal ? 'コピーした変換文の全文を見る' : 'テスト用コピーの全文を見る'}</summary><pre>{snapshot.testCopy || snapshot.transformed}</pre></details><div className="paper-action"><button className="primary-button" onClick={openAnswer}>AIの回答を貼り付け<Arrow/></button></div><p className="paper-footnote centered">実際のAIへの貼り付け・送信は利用者が行います。ここでは架空の回答を使えます。</p></> : <>
            <div className="paper-heading"><span className="paper-kicker">相談1に戻す</span><h2>{snapshot.phase === 'RESTORED_LOCAL' && !editingAnswer ? 'このPC内で復元しました。' : 'AIの回答を貼り付け'}</h2></div>
            {snapshot.phase === 'RESTORED_LOCAL' && !editingAnswer ? <><div className="local-result-label">復元済み回答・このPC内だけ</div><pre className="restored-text" tabIndex={0} aria-label="復元済み回答">{snapshot.restored}</pre>{snapshot.unknownTokens.length > 0 && <div className="stop-note" role="status"><strong>復元できないトークンがあります。</strong><p>未知・変更済み・別の相談のトークンは、そのまま残しました。</p><ul>{snapshot.unknownTokens.map((token, index) => <li className="token-wrap" key={index}>{token}</li>)}</ul></div>}<p className="local-warning">具体的な名前が戻っています。復元済み回答を外部AIへ再び貼り付けないでください。</p><p className="paper-footnote">トークン以外の文章の正しさは確認していません。内容もご自身で確かめてください。</p><div className="closing-note"><LeafMark/><span>ここで閉じても、大丈夫。<br/><small>{native ? '保存は設定から明示操作したときだけ。' : 'この試作は内容を保存しません。'}</small></span></div><button className="text-button" onClick={() => { setEditingAnswer(true); requestAnimationFrame(() => answerRef.current?.focus()); }}>回答を修正する</button></> : <>
              <p className="field-help">この相談でコピーした登録トークンだけを、完全一致で戻します。</p><label className="sr-only" htmlFor="answer-input">AIの回答</label><textarea ref={answerRef} id="answer-input" className="answer-text" value={answer} onChange={event => setAnswer(event.target.value)} placeholder="利用者がコピーした回答を、ここへ貼り付けてください。" spellCheck={false} autoComplete="off"/><div className="answer-tools"><button className="text-button" onClick={loadAnswerSample}>架空の回答を入れる</button><span>HTML・画像・リンクは実行しません。</span></div><div className="paper-action"><button className="primary-button" disabled={!answer.trim() || busy} onClick={() => runAction({ op: 'restore', answer }, () => setEditingAnswer(false))}>このPC内で復元<Arrow/></button></div>
            </>}
          </>}
        </section> : <section className="paper writing-paper">
          <div className="paper-topline"><span><span className="handdrawn-dash" aria-hidden="true"/> 原文の便箋</span><span>外へは渡しません</span></div>
          {isBlocked && <div className="stop-note" role="alert"><strong>{isLocal && model?.state === 'unavailable' ? 'ローカルモデルの準備が必要です。' : '変換文をコピーできません。'}</strong><p>{snapshot.blockedReason}</p><p>原文はそのまま残っています。停止理由とモデルの状態を確かめて、修正してからもう一度確認してください。</p></div>}
          {isScanning && <div className="scan-status" role="status"><span className="scan-mark" aria-hidden="true"/><div><strong>{isLocal ? 'ローカルモデルで機密候補を調べています。' : heldJob ? 'デモの検査結果の受け取りを待っています。' : '辞書と変換候補を確認しています。'}</strong><span>{!isLocal && heldJob ? '設定で手動受け取りを選択しています。' : '処理中も原文はここに残ります。コピーはできません。'}</span></div><button className="text-button" onClick={() => { ++epoch.current; busyRef.current = false; runAction({ op: 'cancel' }); setHeldJob(null); }}>キャンセル</button></div>}
          <label className="sr-only" htmlFor="original-input">原文</label><textarea ref={originalRef} id="original-input" className="original-input" aria-describedby="input-limit input-boundary" value={snapshot.original} onChange={event => edit({ original: event.target.value })} placeholder={'書きかけの文章も、\nまだ形にならないメモも。'} spellCheck={false} autoComplete="off"/>
          <div className="input-tools"><div className="input-tool-buttons"><button className="text-button" onClick={loadSample}><span aria-hidden="true">＋</span> 架空のサンプルを入れる</button>{native && <><button className="text-button" disabled={busy || isScanning} onClick={() => fileRef.current?.click()}>TXTを読み込む</button><input ref={fileRef} type="file" accept=".txt,text/plain" className="sr-only" aria-label="UTF-8 TXTを選択" onChange={event => importText(event.target.files?.[0])}/></>}</div><span id="input-limit">{native ? 'TXTはUTF-8・64 KiBまで · 自動保存なし' : '短いテキストのみ · 保存しません'}</span></div>
          <fieldset className="purpose-field"><legend>どのように手伝ってもらいますか</legend><div className="purpose-options">{PURPOSES.map(([value, label]) => <label className={snapshot.purpose === value ? 'selected' : ''} key={value}><input type="radio" name="purpose" value={value} checked={snapshot.purpose === value} onChange={() => edit({ purpose: value })}/><span>{label}</span></label>)}</div></fieldset>
          <div className="paper-bottom"><p id="input-boundary">{isLocal ? '実推論の試作です。持ち出しは内蔵の架空サンプルだけに制限しています。実在の情報は入れないでください。' : 'まずは架空のサンプルで。\n実在の名前や機密情報は入れないでください。'}</p>{isScanning && !isLocal && heldJob ? <button className="primary-button" onClick={() => finishScan(heldJob, epoch.current)}>デモ結果を受け取る<Arrow/></button> : <button className="primary-button" disabled={!snapshot.original.trim() || busy || isScanning} onClick={scan}>機密箇所を確認・変換<Arrow/></button>}</div>
          {isBlocked && isLocal && <button className="text-button back-to-demo" onClick={() => edit({ mode: 'demo' })}>架空サンプルのデモに戻る</button>}
        </section>}
        <div className="after-paper"><span role="status">{notice}</span>{(isReview || isExported) && <button className="text-button" onClick={() => { setConfirmed(false); runAction({ op: 'cancel' }, () => { setAnswerOpen(false); setEditingAnswer(false); }); }}><Arrow back/> 原文の編集に戻る</button>}</div>
      </>}
      <footer className="room-footer"><p>この部屋の中で変換。外へ渡すのは、あなたの確認のあと。</p><a className="embedded-entry-link" href="./embedded.html">{native ? '組み込みチャットを開く' : '組み込みチャットの接続デモを見る'} <span aria-hidden="true">↗</span></a><details><summary>この試作でできること・できないこと</summary><p>{native ? '端末内のモデルで候補を抽出し、辞書と照合して変換します。現在は内蔵の架空サンプルを検査・承認した変換文だけをOSクリップボードへコピーできます。業務利用の受入基準は未達です。AIへの貼り付け・送信はご自身で行います。TXT読込と暗号化保存はこのPC内で扱います。モデル・保存基盤が未準備の場合は、その操作を止めます。' : 'ブラウザは架空データのデモ専用です。辞書照合・変換・確認・テスト用コピー・復元を体験できます。実モデル、TXT読込、OSコピー、暗号化保存はデスクトップ版で扱います。'}</p><p>クラウドIME、画面記録、クリップボード同期などはアプリ外の経路です。このアプリがPC全体を監視・封鎖することはありません。「包む」という表現は仮名化の比喩で、暗号文をAIへ送る意味ではありません。</p></details></footer>
    </main>
    {companion && <aside className="companion" aria-hidden="true"><img src="/art/companion-idle.png" alt=""/><span>ポッケ</span></aside>}
    <dialog ref={dialogRef} className="settings-drawer" onCancel={event => { event.preventDefault(); closeSettings(); }} onClose={() => setSettings(false)} aria-labelledby="settings-title"><div className="drawer-heading"><div><span className="eyebrow">THE LITTLE DRAWER</span><h2 id="settings-title">部屋の設定</h2></div><button className="close-button" aria-label="設定を閉じる" onClick={closeSettings}>×</button></div><section className="drawer-section"><h3>あなたの読みやすさで</h3><label className="setting-check"><input type="checkbox" checked={quiet} onChange={event => setQuiet(event.target.checked)}/><span>静かな表示<small>背景と小物を控えめにします。</small></span></label><label className="setting-check"><input type="checkbox" checked={motionOff} onChange={event => setMotionOff(event.target.checked)}/><span>動きを止める<small>操作や確認の内容は変わりません。</small></span></label><label className="setting-check"><input type="checkbox" checked={companion} onChange={event => setCompanion(event.target.checked)}/><span>ポッケを表示する</span></label><p className="field-help">音はOFFです。この試作は音を再生しません。</p></section>
      {snapshot && <>
        <section className="drawer-section">
          <h3>動作とモデル</h3>
          <label className="select-setting">実行モード
            <select value={snapshot.mode} onChange={event => { edit({ mode: event.target.value }); refreshModel(); }}>
              <option value="demo">デモ・架空データのみ</option>
              <option value="local" disabled={!native}>ローカル実推論・このPC内</option>
            </select>
          </label>
          <p className="field-help">{native ? '実推論はこのPC内で行います。デモは架空の検査結果を使い、OSクリップボードへ書き込みません。' : 'ブラウザは架空データのデモ専用です。実モデルはデスクトップアプリで扱います。'}</p>
          <div className="model-status-note" role="status">
            <strong>{model?.state === 'ready' ? 'モデルの準備ができています' : model?.state === 'busy' ? 'モデルを実行しています' : model?.state === 'verifying' ? 'モデルを確認しています' : model?.state === 'error' ? 'モデルの動作を止めています' : 'モデルがまだ準備できていません'}</strong>
            <p>{model?.modelName || '状態を確認中'}</p>
            {(modelError || model?.reason) && <p>{modelError || model?.reason}</p>}
            {native && <><p>出所・hashの確認：{model?.sourceVerified ? 'ホストが確認済み' : '未確認'}<br/>推論プロセスの通信制限：{model?.networkIsolated ? '有効' : '未確認'}</p><div className="model-actions"><button className="text-button" onClick={refreshModel}>モデルの状態を確認する</button><button className="text-button" disabled={busy || isScanning || model?.state === 'busy' || model?.state === 'verifying'} onClick={prepareModel}>モデルを再準備する</button></div></>}
          </div>
        </section>
        <section className="drawer-section">
          <h3>このPCの暗号化保存</h3>
          {!native ? <p className="save-note">ブラウザでは保存できません。相談はメモリ内だけで扱います。</p> : <>
            <p className="field-help">保存は明示操作のときだけ行います。平文では保存しません。保存した相談の承認は復帰せず、持ち出す前に再検査します。</p>
            {!vault?.unlocked ? <>
              <p className="save-note">{vault?.available ? '暗号化保存は閉じています。' : vault?.reason || '保存基盤の状態を確認しています。'}</p>
              <button className="text-button" disabled={!vault?.available || vaultBusy} onClick={() => vaultAction('unlock')}>暗号化保存を開く</button>
            </> : <>
              <div className="vault-actions"><button className="vault-save-button" disabled={vaultBusy || busy || isScanning || Boolean(error) || !snapshot.original.trim()} onClick={() => vaultAction('save')}>この相談を暗号化して保存</button><button className="text-button" disabled={vaultBusy} onClick={() => vaultAction('lock')}>保存をロック</button></div>
              {vaultEntries.length > 0 ? <>
                <label className="select-setting">保存した相談
                  <select value={selectedSave} disabled={vaultBusy} onChange={event => { setSelectedSave(event.target.value); setDeleteConfirmed(false); }}>
                    <option value="">保存日時を選択</option>
                    {vaultEntries.map(entry => <option key={entry.id} value={entry.id}>{new Date(entry.createdMs).toLocaleString('ja-JP')} · {entry.id.slice(-6)}</option>)}
                  </select>
                </label>
                <button className="text-button" disabled={!selectedSave || vaultBusy || busy || isScanning} onClick={() => vaultAction('load')}>選択した保存を開く</button>
                <p className="field-help">現在の未保存の相談を置き換えます。保存名に原文や人名は使いません。</p>
                <details className="vault-delete"><summary>選択した保存を削除する</summary><label className="setting-check"><input type="checkbox" checked={deleteConfirmed} disabled={!selectedSave} onChange={event => setDeleteConfirmed(event.target.checked)}/><span>選択した保存を削除してよいことを確認しました。</span></label><button className="text-button" disabled={!selectedSave || !deleteConfirmed || vaultBusy} onClick={() => vaultAction('delete')}>この保存を削除</button></details>
              </> : <p className="field-help">保存した相談はまだありません。</p>}
            </>}
            {vaultNotice && <p className="vault-feedback" role="status">{vaultNotice}</p>}
            {vaultError && <p className="vault-error" role="alert">{vaultError}</p>}
          </>}
        </section>
        <details className="technical-settings"><summary>検査の構成とデモ試験</summary><div className="technical-inner">
          <label>秘密辞書（このPC内）<textarea value={snapshot.dictionaryText} onChange={event => edit({ dictionaryText: event.target.value })} spellCheck={false}/></label>
          <label>ポリシー版<input value={snapshot.policyVersion} onChange={event => edit({ policyVersion: event.target.value })}/></label>
          <label>モデル構成{isLocal && '（ホストの検証値）'}<input value={isLocal ? model?.modelConfig || snapshot.modelConfig : snapshot.modelConfig} readOnly={isLocal} onChange={event => edit({ modelConfig: event.target.value })}/></label>
          <p className="field-help">辞書・ポリシー・モデル構成を変更すると、確認と承認は無効になります。</p>
          {snapshot.mode === 'demo' && <><label>デモの失敗状態<select aria-label="デモの失敗状態" value={fault} onChange={event => setFault(event.target.value as Fault)}><option value="none">通常のデモ</option><option value="invalid_json">JSONが壊れた場合</option><option value="stopped">モデルが停止した場合</option><option value="incomplete">未処理の部分がある場合</option></select></label><label className="setting-check"><input type="checkbox" checked={holdScan} onChange={event => setHoldScan(event.target.checked)}/><span>デモ結果の受け取りを手動にする<small>処理中の編集・キャンセルを確かめるための試験設定です。通常処理に待ち時間はありません。</small></span></label>{heldJob && !isScanning && <button className="text-button" onClick={() => finishScan(heldJob, epoch.current)}>古いデモ結果を受け取る（試験）</button>}</>}
        </div></details>
        <section className="drawer-section"><button className="text-button" onClick={() => { restart(); closeSettings(); }}>この相談を閉じて、空の便箋に戻る</button><p className="field-help">原文と対応表を次の相談へ引き継ぎません。明示的に保存した相談は削除されません。</p></section>
      </>}
    </dialog>
  </div>;
}
