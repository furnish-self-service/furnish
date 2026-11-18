// Plan a whole request before starting its first install, so every dependent has a
// record of the operations that must succeed before it can run.
import type { AppInfo, Trigger } from './api';

export interface Queued {
  key: string;
  action: 'install' | 'uninstall' | 'reinstall' | 'installPrevious' | 'installCurrent' | 'installVersion';
  commit?: string;
  trigger: Trigger;
  prerequisites: string[];
}

type App = Pick<AppInfo, 'key' | 'dependencies'>;
export const defaultDependencies = (app: App) =>
  app.dependencies.filter(dep => dep.required || dep.selected).map(dep => dep.key);

export function planInstall(
  root: App, chosen: string[], catalog: App[], pending: Queued[], satisfied: (key: string) => boolean,
) {
  const available = new Map(catalog.map(app => [app.key, app]));
  const existing = new Map(pending.map(job => [job.key, job]));
  const awaited = new Set(pending.flatMap(job => job.prerequisites));
  const path = new Set<string>();
  const members = new Set<string>();
  const installs: { key: string; prerequisites: string[] }[] = [];
  let error: { kind: 'cycle' | 'missing' | 'uninstalling'; detail: string } | undefined;

  function visit(key: string, schedule: boolean) {
    if (error) return;
    if (path.has(key)) {
      error = { kind: 'cycle', detail: [...path, key].join(' → ') };
      return;
    }
    if (members.has(key)) return;
    path.add(key);
    const job = existing.get(key);
    if (job?.action === 'uninstall') {
      error = { kind: 'uninstalling', detail: key };
      return;
    }
    if (job) {
      // Joining shared work preserves its original choices. A failed prerequisite
      // stays blocked until the user retries it.
      for (const dep of job.prerequisites) visit(dep, false);
    } else if (schedule && (key === root.key || !awaited.has(key))) {
      const app = available.get(key);
      if (!app) {
        error = { kind: 'missing', detail: key };
        return;
      }
      const keys = key === root.key ? [...app.dependencies.filter(d => d.required).map(d => d.key), ...chosen] : defaultDependencies(app);
      const prerequisites = [...new Set(keys)].filter(dep => existing.has(dep) || !satisfied(dep));
      for (const dep of prerequisites) visit(dep, true);
      installs.push({ key, prerequisites });
    }
    path.delete(key);
    members.add(key);
  }

  visit(root.key, true);
  return { installs: error ? [] : installs, members, error };
}

/** A failed or removed request leaves its dependents waiting. Only a successful
 *  install (including a retry) releases them; unrelated jobs remain runnable. */
export function releaseDependents(queue: Queued[], installed: string) {
  for (const job of queue) {
    const index = job.prerequisites.indexOf(installed);
    if (index !== -1) job.prerequisites.splice(index, 1);
  }
}
