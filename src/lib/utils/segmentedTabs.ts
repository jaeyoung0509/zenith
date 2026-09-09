interface SegmentedTabItem {
  id: string;
}

export function handleSegmentedTabKeydown(
  event: KeyboardEvent,
  currentIndex: number,
  tabs: readonly SegmentedTabItem[],
  tablist: Pick<HTMLElement, 'querySelectorAll'> | null,
  onSelect: (id: string) => void,
) {
  if (tabs.length === 0) return;

  let nextIndex = currentIndex;
  if (event.key === 'ArrowRight') {
    nextIndex = (currentIndex + 1) % tabs.length;
  } else if (event.key === 'ArrowLeft') {
    nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
  } else if (event.key === 'Home') {
    nextIndex = 0;
  } else if (event.key === 'End') {
    nextIndex = tabs.length - 1;
  } else {
    return;
  }

  event.preventDefault();
  const nextTab = tabs[nextIndex];
  if (!nextTab) return;

  onSelect(nextTab.id);
  tablist?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[nextIndex]?.focus();
}
