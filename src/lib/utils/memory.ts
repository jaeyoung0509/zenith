import type { ProcessMemory } from '../models/types';

/**
 * Filter a list of top processes case-insensitively by process/app name, any constituent
 * PID substring, or a parent process name (so "everything Warp started" is searchable).
 * Empty queries return the entire list untouched.
 */
export function filterProcesses(processes: ProcessMemory[], query: string): ProcessMemory[] {
  if (!processes || processes.length === 0) return [];
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return processes;

  return processes.filter((proc) => {
    const nameMatch = proc.name.toLowerCase().includes(trimmed);
    const pidMatch =
      String(proc.pid).includes(trimmed) ||
      (Array.isArray(proc.pids) && proc.pids.some((p) => String(p).includes(trimmed)));
    const parentMatch =
      Array.isArray(proc.parent_process_names) &&
      proc.parent_process_names.some((parent) => parent.toLowerCase().includes(trimmed));
    return nameMatch || pidMatch || parentMatch;
  });
}
