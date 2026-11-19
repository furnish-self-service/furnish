import type { DesktopUpdateStatus } from './api';

export interface RestartState {
  installedDesktop: DesktopUpdateStatus['installed'];
  desktopRestart: 'idle' | 'checking' | 'launching';
}

/** Polls describe the executable on disk. Only a user click may launch it. */
export function desktopRestart(
  state: RestartState, idle: () => boolean,
  check: () => Promise<DesktopUpdateStatus>, launch: (expected: NonNullable<DesktopUpdateStatus['installed']>) => Promise<boolean>,
) {
  let generation = 0;
  let polling: number | undefined;

  async function refresh() {
    if (polling !== undefined || state.desktopRestart !== 'idle') return;
    const current = ++generation;
    polling = current;
    try {
      const status = await check();
      if (current === generation) state.installedDesktop = status.changed ? status.installed : null;
    } catch {
      if (current === generation) state.installedDesktop = null;
    } finally {
      if (polling === current) polling = undefined;
    }
  }

  async function restart() {
    if (state.desktopRestart !== 'idle' || !state.installedDesktop) return;
    if (!idle()) return 'busy';
    generation++; // A poll that started before the click cannot restore an obsolete offer.
    polling = undefined;
    const offered = state.installedDesktop;
    state.desktopRestart = 'checking';
    let launched = false;
    try {
      const status = await check();
      const installed = status.changed ? status.installed : null;
      state.installedDesktop = installed;
      if (!installed) return 'unavailable';
      // If a package manager replaced it again, let the user choose the newly displayed build.
      if (installed.version !== offered.version || installed.revision !== offered.revision) return;
      if (!idle()) return 'busy';
      state.desktopRestart = 'launching';
      launched = await launch(installed);
      if (!launched) return 'postponed';
    } finally {
      if (!launched) state.desktopRestart = 'idle';
    }
  }

  return { refresh, restart };
}
