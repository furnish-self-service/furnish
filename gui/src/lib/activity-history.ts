// The desktop's recent operations. Only these summary fields go to disk: commands,
// command output, errors and configuration diffs stay in the running webview.
import type { LogLine, Outcome, Progress, Step, Trigger } from './api';

export const historyLimit = 30;
export type OperationKind = 'install' | 'update' | 'uninstall' | 'reinstall';
type Result = 'done' | 'failed' | 'cancelled' | 'fallback';

export interface RunStep {
  step: Step;
  state: 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'cancelled' | 'notRun';
  startedAt?: number;
  durationMs?: number;
  outcome?: Outcome;
}

export interface Run {
  id: string;
  name: string;
  kind: OperationKind;
  trigger: Trigger;
  startedAt: number;
  finishedAt?: number;
  stopping?: 'abort' | 'force';
  result?: Result;
  error?: string;
  fallbackDate?: string;
  steps: RunStep[];
  /** Engine indices restart at zero for the second phase of a reinstall or fallback. */
  phaseStart: number;
}

export function recordProgress(run: Run, p: Progress) {
  if (p.event === 'planned') {
    for (const row of run.steps.slice(run.phaseStart)) {
      if (row.state === 'pending') row.state = 'notRun';
    }
    run.phaseStart = run.steps.length;
    for (const step of p.steps) run.steps.push({ step, state: 'pending' });
    return;
  }
  const row = run.steps[run.phaseStart + p.index];
  if (!row) return;
  if (p.event === 'started') {
    row.state = 'running';
    row.startedAt = p.atMs;
    return;
  }
  row.state = p.outcome.cancelled ? 'cancelled' : p.outcome.error ? 'failed' : p.outcome.action === 'skipped' ? 'skipped' : 'done';
  row.startedAt ??= p.atMs - p.durationMs;
  row.durationMs = p.durationMs;
  row.outcome = p.outcome;
}

export interface Summary {
  id: string;
  key: string;
  name: string;
  kind: OperationKind;
  trigger: Trigger;
  finishedAt: number;
  result: Result;
  failedStep: string | null;
  fallbackDate: string | null;
}

export interface RecentActivity {
  summary: Summary;
  details?: { run: Run; logs: LogLine[] };
}

export function completedActivity(key: string, run: Run, logs: LogLine[]): RecentActivity {
  if (!run.result || run.finishedAt === undefined) throw new Error('Operation has not finished');
  const failed = run.steps.slice(run.phaseStart).find(step => step.state === 'failed');
  return {
    summary: {
      id: run.id, key, name: run.name, kind: run.kind, trigger: run.trigger,
      finishedAt: run.finishedAt, result: run.result,
      failedStep: run.result === 'failed' ? failed?.step.name ?? null : null,
      fallbackDate: run.fallbackDate ?? null,
    },
    details: { run, logs },
  };
}

function isSummary(v: unknown): v is Summary {
  if (!v || typeof v !== 'object') return false;
  const s = v as Record<string, unknown>;
  return typeof s.id === 'string' && typeof s.name === 'string' && typeof s.key === 'string'
    && typeof s.kind === 'string' && ['install', 'update', 'uninstall', 'reinstall'].includes(s.kind)
    && typeof s.trigger === 'string' && ['user', 'update', 'autoInstall'].includes(s.trigger)
    && typeof s.result === 'string' && ['done', 'failed', 'cancelled', 'fallback'].includes(s.result)
    && typeof s.finishedAt === 'number' && Number.isFinite(new Date(s.finishedAt).getTime())
    && (s.failedStep === null || typeof s.failedStep === 'string')
    && (s.fallbackDate === null || typeof s.fallbackDate === 'string');
}

export type KitItemState = 'queued' | 'running' | Result | 'interrupted';

export interface KitItem {
  key: string;
  name: string;
  /** Other items are prerequisites added by the install planner. */
  selected: boolean;
  state: KitItemState;
  attempts: RecentActivity[];
  /** Outstanding prerequisites of this request when its queue entry was last saved. */
  waitingFor?: string[];
  /** Current session only: a shared pre-existing job also keeps its own Activity row. */
  ownsJob?: boolean;
  runId?: string;
}

export interface KitActivity {
  id: string;
  key: string;
  name: string;
  updatedAt: number;
  items: KitItem[];
}

export type ActivityEntry = RecentActivity | KitActivity;
export function isKit(entry: ActivityEntry): entry is KitActivity {
  return 'items' in entry;
}
export const kitItemActive = (item: KitItem) => item.state === 'queued' || item.state === 'running';
export const kitItemDone = (item: KitItem) => item.state === 'done' || item.state === 'fallback';

/** Retry only the branch interrupted behind the repaired prerequisite. Failed,
 *  explicitly cancelled and independently interrupted apps require their own retry. */
export function interruptedDependents(kit: KitActivity, repaired: string): KitItem[] {
  const resumed = new Set([repaired]);
  const items: KitItem[] = [];
  for (const key of resumed) {
    for (const item of kit.items) {
      if (item.state === 'interrupted' && !resumed.has(item.key) && item.waitingFor?.includes(key)) {
        resumed.add(item.key);
        items.push(item);
      }
    }
  }
  return items;
}

/** Never discard a user request while one of its apps still waits for a prerequisite. */
export function trimHistory(entries: ActivityEntry[]) {
  let finished = 0;
  return entries.filter(e => (isKit(e) && e.items.some(kitItemActive)) || ++finished <= historyLimit);
}

function readKit(record: Record<string, unknown>): KitActivity | undefined {
  if (typeof record.id !== 'string' || typeof record.key !== 'string' || typeof record.name !== 'string'
    || typeof record.updatedAt !== 'number' || !Number.isFinite(new Date(record.updatedAt).getTime())
    || !Array.isArray(record.items)) return;
  const items: KitItem[] = [];
  for (const item of record.items) {
    if (!item || typeof item.key !== 'string' || typeof item.name !== 'string' || typeof item.selected !== 'boolean'
      || !['queued', 'running', 'done', 'failed', 'cancelled', 'fallback', 'interrupted'].includes(item.state)
      || !Array.isArray(item.attempts) || (item.waitingFor !== undefined &&
        (!Array.isArray(item.waitingFor) || !item.waitingFor.every((key: unknown) => typeof key === 'string')))) return;
    items.push({
      key: item.key, name: item.name, selected: item.selected,
      state: kitItemActive(item) ? 'interrupted' : item.state,
      waitingFor: item.waitingFor,
      attempts: item.attempts.filter(isSummary).slice(0, historyLimit).map((summary: Summary) => ({ summary })),
    });
  }
  if (items.length) return { id: record.id, key: record.key, name: record.name, updatedAt: record.updatedAt, items };
}

const storageKey = (dataDir: string) => 'furnish.activity.v1:' + dataDir;

export function readHistory(dataDir: string): ActivityEntry[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey(dataDir)) ?? '[]');
    if (!Array.isArray(saved)) return [];
    const entries: ActivityEntry[] = [];
    for (const value of saved) {
      if (isSummary(value)) entries.push({ summary: value });
      else if (value?.kind === 'kit') {
        const kit = readKit(value);
        if (kit) entries.push(kit);
      }
    }
    return trimHistory(entries);
  } catch { /* Private or damaged storage: this session still has history. */ }
  return [];
}

export function writeHistory(dataDir: string, entries: ActivityEntry[]) {
  try {
    const saved = trimHistory(entries).map(e => isKit(e) ? {
      kind: 'kit', id: e.id, key: e.key, name: e.name, updatedAt: e.updatedAt,
      items: e.items.map(({ key, name, selected, state, attempts, waitingFor }) =>
        ({ key, name, selected, state, waitingFor, attempts: attempts.map(a => a.summary) })),
    } : e.summary);
    localStorage.setItem(storageKey(dataDir), JSON.stringify(saved));
  } catch { /* Storage may be full or disabled. */ }
}
