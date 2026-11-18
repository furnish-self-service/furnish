// The desktop's recent operations. Only these summary fields go to disk: commands,
// command output, errors and configuration diffs stay in the running webview.
import type { LogLine, Outcome, Progress, Step, Trigger } from './api';

import type { Queued } from './install-queue';

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
  collectionRevision?: string;
  action?: Queued['action'];
  commit?: string;
  requiresNewSession?: boolean;
  fallbackDate?: string;
  steps: RunStep[];
  /** Engine indices restart at zero for the second phase of a reinstall or fallback. */
  phaseStart: number;
}

export function recordProgress(run: Run, p: Progress) {
  if (p.event === 'revision') {
    // A fallback is another attempt and can fail before producing its own plan.
    if (run.steps.length) {
      for (const row of run.steps.slice(run.phaseStart)) {
        if (row.state === 'pending') row.state = 'notRun';
      }
      run.phaseStart = run.steps.length;
    }
    run.collectionRevision = p.revision ?? undefined;
    return;
  }
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
  failure?: Failure;
  collectionRevision?: string;
  action?: Queued['action'];
  commit?: string;
  steps?: { name: string; state: RunStep['state'] }[];
  requiresNewSession?: boolean;
  fallbackDate: string | null;
}

export interface RecentActivity {
  summary: Summary;
  details?: { run: Run; logs: LogLine[] };
}

const failureCodes = ['permission', 'answers', 'verification', 'package', 'step', 'operation'] as const;
type Failure = typeof failureCodes[number];

/** Error messages may contain command output, credentials, and option values.
 * Only a fixed diagnostic category crosses the persistence boundary. */
function failureReason(run: Run, failed?: RunStep): Failure {
  const message = failed?.outcome?.error ?? run.error ?? '';
  if (/permission denied|access (?:is )?denied|operation not permitted/i.test(message)) return 'permission';
  if (message.includes('first needs your answer')) return 'answers';
  if (message.includes("isn't installed after the install")) return 'verification';
  if (failed?.step.type.toLowerCase() === 'furnish/package') return 'package';
  return failed ? 'step' : 'operation';
}

const isRevision = (value: unknown): value is string =>
  typeof value === 'string' && /^[a-f0-9]{4,64}$/i.test(value);

export function completedActivity(key: string, run: Run, logs: LogLine[]): RecentActivity {
  if (!run.result || run.finishedAt === undefined) throw new Error('Operation has not finished');
  const failed = run.steps.slice(run.phaseStart).find(step => step.state === 'failed');
  return {
    summary: {
      id: run.id, key, name: run.name, kind: run.kind, trigger: run.trigger,
      finishedAt: run.finishedAt, result: run.result,
      failedStep: run.result === 'failed' ? failed?.step.name ?? null : null,
      failure: run.result === 'failed' ? failureReason(run, failed) : undefined,
      collectionRevision: isRevision(run.collectionRevision) ? run.collectionRevision : undefined,
      action: run.action, commit: isRevision(run.commit) ? run.commit : undefined,
      steps: run.steps.map(row => ({ name: row.step.name, state: row.state === 'pending' ? 'notRun' : row.state })),
      requiresNewSession: (run.result === 'done' || run.result === 'fallback') && run.requiresNewSession || undefined,
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
    && (s.failure === undefined || failureCodes.some(code => code === s.failure))
    && (s.collectionRevision === undefined || isRevision(s.collectionRevision))
    && (s.commit === undefined || isRevision(s.commit))
    && (s.action === undefined || ['install', 'reinstall', 'uninstall', 'installVersion', 'installCurrent', 'installPrevious'].some(action => action === s.action))
    && (s.steps === undefined || (Array.isArray(s.steps) && s.steps.every(step => step && typeof step.name === 'string'
      && ['pending', 'running', 'done', 'failed', 'skipped', 'cancelled', 'notRun'].includes(step.state))))
    && (s.failedStep === null || typeof s.failedStep === 'string')
    && (s.requiresNewSession === undefined || typeof s.requiresNewSession === 'boolean')
    && (s.fallbackDate === null || typeof s.fallbackDate === 'string');
}

export type GroupItemState = 'queued' | 'running' | Result | 'interrupted' | 'skipped';

export interface GroupItem {
  key: string;
  name: string;
  /** Other items are prerequisites added by the install planner. */
  selected: boolean;
  state: GroupItemState;
  attempts: RecentActivity[];
  /** Outstanding prerequisites of this request when its queue entry was last saved. */
  waitingFor?: string[];
  /** Current session only: a shared pre-existing job also keeps its own Activity row. */
  ownsJob?: boolean;
  runId?: string;
}

export type ActivityGroup = ({ kind: 'kit'; key: string; name: string } | { kind: 'updates' }) & {
  id: string;
  updatedAt: number;
  items: GroupItem[];
}

export type ActivityEntry = RecentActivity | ActivityGroup;
export function isGroup(entry: ActivityEntry): entry is ActivityGroup {
  return 'items' in entry;
}
export const itemActive = (item: GroupItem) => item.state === 'queued' || item.state === 'running';
export const itemDone = (item: GroupItem) => item.state === 'done' || item.state === 'fallback';

/** Follow queue edges to the prerequisite that needs intervention. An app
 * waiting behind another healthy job still counts as waiting. After a restart
 * the saved edges explain which interrupted branches were blocked. */
export function groupProgress(group: ActivityGroup, queue: Queued[], pending: (key: string) => boolean) {
  const members = new Map(group.items.map(item => [item.key, item]));
  const jobs = new Map(queue.map(job => [job.key, job]));
  function blockers(key: string, path: Set<string>): string[] {
    if (path.has(key)) return [];
    const item = members.get(key);
    if (item?.state !== 'queued' && item?.state !== 'interrupted') return [];
    const next = new Set([...path, key]);
    const edges = jobs.get(key)?.prerequisites ?? item.waitingFor ?? [];
    return edges.flatMap(dep => {
      const deeper = blockers(dep, next);
      if (deeper.length) return deeper;
      if (pending(dep) || members.get(dep)?.state === 'done' || members.get(dep)?.state === 'fallback') return [];
      return [dep];
    });
  }
  const selected = group.items.filter(i => i.selected);
  const blocked = new Map<string, string[]>();
  for (const item of group.items) {
    const keys = [...new Set(blockers(item.key, new Set()))];
    if (keys.length) blocked.set(item.key, keys);
  }
  return {
    blocked,
    waiting: group.items.filter(i => i.state === 'queued' && !blocked.has(i.key)).length,
    done: selected.filter(i => group.kind === 'updates' ? i.state === 'done' : itemDone(i)).length,
    total: selected.length,
  };
}

/** Retry only the branch interrupted behind the repaired prerequisite. Failed,
 *  explicitly cancelled and independently interrupted apps require their own retry. */
export function interruptedDependents(group: ActivityGroup, repaired: string): GroupItem[] {
  const resumed = new Set([repaired]);
  const items: GroupItem[] = [];
  for (const key of resumed) {
    for (const item of group.items) {
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
  return entries.filter(e => (isGroup(e) && e.items.some(itemActive)) || ++finished <= historyLimit);
}

function readGroup(record: Record<string, unknown>): ActivityGroup | undefined {
  if (typeof record.id !== 'string' || (record.kind !== 'updates' && (record.kind !== 'kit' || typeof record.key !== 'string' || typeof record.name !== 'string'))
    || typeof record.updatedAt !== 'number' || !Number.isFinite(new Date(record.updatedAt).getTime())
    || !Array.isArray(record.items)) return;
  const items: GroupItem[] = [];
  for (const item of record.items) {
    if (!item || typeof item.key !== 'string' || typeof item.name !== 'string' || typeof item.selected !== 'boolean'
      || !['queued', 'running', 'done', 'failed', 'cancelled', 'fallback', 'interrupted', 'skipped'].includes(item.state)
      || !Array.isArray(item.attempts) || (item.waitingFor !== undefined &&
        (!Array.isArray(item.waitingFor) || !item.waitingFor.every((key: unknown) => typeof key === 'string')))) return;
    items.push({
      key: item.key, name: item.name, selected: item.selected,
      state: itemActive(item) ? 'interrupted' : item.state,
      waitingFor: item.waitingFor,
      attempts: item.attempts.filter(isSummary).slice(0, historyLimit).map((summary: Summary) => ({ summary })),
    });
  }
  if (!items.length) return;
  const group = { id: record.id, updatedAt: record.updatedAt, items };
  if (record.kind === 'updates') return { ...group, kind: 'updates' };
  if (typeof record.key === 'string' && typeof record.name === 'string')
    return { ...group, kind: 'kit', key: record.key, name: record.name };
}

const storageKey = (dataDir: string) => 'furnish.activity.v1:' + dataDir;

export function readHistory(dataDir: string): ActivityEntry[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey(dataDir)) ?? '[]');
    if (!Array.isArray(saved)) return [];
    const entries: ActivityEntry[] = [];
    for (const value of saved) {
      if (isSummary(value)) entries.push({ summary: value });
      else if (value?.kind === 'kit' || value?.kind === 'updates') {
        const group = readGroup(value);
        if (group) entries.push(group);
      }
    }
    return trimHistory(entries);
  } catch { /* Private or damaged storage: this session still has history. */ }
  return [];
}

export function writeHistory(dataDir: string, entries: ActivityEntry[]) {
  try {
    const saved = trimHistory(entries).map(e => isGroup(e) ? {
      kind: e.kind, id: e.id, updatedAt: e.updatedAt,
      ...(e.kind === 'kit' ? { name: e.name, key: e.key } : {}),
      items: e.items.map(({ key, name, selected, state, attempts, waitingFor }) =>
        ({ key, name, selected, state, waitingFor, attempts: attempts.map(a => a.summary) })),
    } : e.summary);
    localStorage.setItem(storageKey(dataDir), JSON.stringify(saved));
  } catch { /* Storage may be full or disabled. */ }
}
