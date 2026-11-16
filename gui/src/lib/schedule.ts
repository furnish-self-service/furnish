// When furnish asks the collections' servers whether they have something new, without applying
// it (the "update available" marks; see check_collection_servers). Cheap: nothing is downloaded.

/** Between update checks further apart than this: look this often. */
export const LOOK_MINUTES = 30;

/**
 * At start (when updating at start is off) and every LOOK_MINUTES between update checks that are
 * further apart. Checking set to Never with updating at start off means no network traffic at all:
 * then the marks only change with Check now or Update.
 */
export function serverLooks(intervalMinutes: number, updateOnLaunch: boolean): { atStart: boolean; everyMinutes: number | null } {
  const never = intervalMinutes === 0;
  const quiet = never && !updateOnLaunch;
  return {
    atStart: !updateOnLaunch && !quiet,
    everyMinutes: quiet || (!never && intervalMinutes <= LOOK_MINUTES) ? null : LOOK_MINUTES,
  };
}
