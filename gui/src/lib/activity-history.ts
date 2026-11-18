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

interface Summary {
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

const storageKey = (dataDir: string) => 'furnish.activity.v1:' + dataDir;

export function readHistory(dataDir: string): RecentActivity[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey(dataDir)) ?? '[]');
    if (Array.isArray(saved)) return saved.filter(isSummary).slice(0, historyLimit).map(summary => ({ summary }));
  } catch { /* Private or damaged storage: this session still has history. */ }
  return [];
}

export function writeHistory(dataDir: string, entries: RecentActivity[]) {
  try {
    localStorage.setItem(storageKey(dataDir), JSON.stringify(entries.slice(0, historyLimit).map(entry => entry.summary)));
  } catch { /* Storage may be full or disabled. */ }
}
