export function formatBytes(bytes: number, decimals: number = 1): string {
  if (bytes === 0) return '0 B';
  if (bytes < 0) return '-' + formatBytes(-bytes, decimals);

  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const clampedI = Math.min(i, sizes.length - 1);

  return `${parseFloat((bytes / Math.pow(k, clampedI)).toFixed(dm))} ${sizes[clampedI]}`;
}

export function formatTimeAgo(timestampSeconds?: number): string {
  if (!timestampSeconds) return 'Never';
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - timestampSeconds);

  if (diff < 60) return 'Just now';
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} hours ago`;
  const days = Math.floor(diff / 86400);
  return `${days} day${days > 1 ? 's' : ''} ago`;
}

export function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  if (mins < 60) return `${mins}m ${secs}s`;
  const hours = Math.floor(mins / 60);
  const remainingMins = mins % 60;
  return `${hours}h ${remainingMins}m`;
}

export function formatTimeUntil(timestampSeconds?: number): string {
  if (!timestampSeconds) return '';
  const now = Math.floor(Date.now() / 1000);
  const diff = timestampSeconds - now;
  if (diff <= 0) return 'now';
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) {
    const hours = Math.floor(diff / 3600);
    const mins = Math.floor((diff % 3600) / 60);
    return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
  }
  const days = Math.floor(diff / 86400);
  const remHours = Math.floor((diff % 86400) / 3600);
  return remHours > 0 ? `${days}d ${remHours}h` : `${days}d`;
}

export function formatResetDate(timestampSeconds?: number): string {
  if (!timestampSeconds) return '';
  return new Intl.DateTimeFormat('en', {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(timestampSeconds * 1000));
}

export function formatCountdown(secs: number): string {
  if (secs <= 0) return 'expired';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m > 0) return `${m}:${String(s).padStart(2, '0')} remaining`;
  return `${s}s remaining`;
}

export function ttlRemaining(expiresAt: number, nowMs: number = Date.now()): number {
  return Math.max(0, expiresAt - Math.floor(nowMs / 1000));
}
