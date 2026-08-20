import type { ProcessMemory } from '../models/types';

/**
 * Filter a list of top processes case-insensitively by process/app name or PID substring.
 * Empty queries return the entire list untouched.
 */
export function filterProcesses(processes: ProcessMemory[], query: string): ProcessMemory[] {
  if (!processes || processes.length === 0) return [];
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return processes;

  return processes.filter((proc) => {
    const nameMatch = proc.name.toLowerCase().includes(trimmed);
    const pidMatch = String(proc.pid).includes(trimmed);
    return nameMatch || pidMatch;
  });
}
