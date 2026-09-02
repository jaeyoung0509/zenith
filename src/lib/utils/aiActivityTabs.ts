export type AiActivitySubTab = 'usage' | 'projects' | 'adapters';

export const AI_ACTIVITY_TAB_ORDER: readonly AiActivitySubTab[] = [
  'usage',
  'projects',
  'adapters',
];

export function nextAiActivityTab(
  currentTab: AiActivitySubTab,
  key: string,
): AiActivitySubTab | null {
  const currentIndex = AI_ACTIVITY_TAB_ORDER.indexOf(currentTab);

  if (key === 'ArrowRight') {
    return AI_ACTIVITY_TAB_ORDER[(currentIndex + 1) % AI_ACTIVITY_TAB_ORDER.length];
  }
  if (key === 'ArrowLeft') {
    return AI_ACTIVITY_TAB_ORDER[
      (currentIndex - 1 + AI_ACTIVITY_TAB_ORDER.length) % AI_ACTIVITY_TAB_ORDER.length
    ];
  }
  if (key === 'Home') return AI_ACTIVITY_TAB_ORDER[0];
  if (key === 'End') return AI_ACTIVITY_TAB_ORDER[AI_ACTIVITY_TAB_ORDER.length - 1];

  return null;
}
