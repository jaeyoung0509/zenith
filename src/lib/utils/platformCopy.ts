/**
 * Backend vocabulary is lower-case prose ("menu bar", "notification area") so
 * it reads correctly inside sentences. Headings need it title-cased without a
 * second hardcoded table drifting from the backend's.
 */
export function titleCaseLabel(label: string): string {
  return label.replace(/(^|\s)\S/g, (character) => character.toUpperCase());
}

/** Copy used when the backend cannot report a log directory at all. */
export const LOG_DIRECTORY_FALLBACK = 'the local log directory';
