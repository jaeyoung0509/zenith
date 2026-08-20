export function toggleOrdered<T>(items: T[], item: T, keepOne = false): T[] {
  if (!items.includes(item)) return [...items, item];
  if (keepOne && items.length === 1) return items;
  return items.filter((candidate) => candidate !== item);
}

export function moveOrdered<T>(items: T[], item: T, direction: -1 | 1): T[] {
  const next = [...items];
  const index = next.indexOf(item);
  const destination = index + direction;
  if (index < 0 || destination < 0 || destination >= next.length) return items;
  [next[index], next[destination]] = [next[destination], next[index]];
  return next;
}

export function reorderOrdered<T>(items: T[], dragged: T, target: T): T[] {
  const next = [...items];
  const from = next.indexOf(dragged);
  const to = next.indexOf(target);
  if (from < 0 || to < 0 || from === to) return items;
  next.splice(from, 1);
  next.splice(to, 0, dragged);
  return next;
}

export function isQuickPanelDismissShortcut(key: string, metaKey: boolean): boolean {
  return key === 'Escape' || (metaKey && key.toLowerCase() === 'w');
}
