import { describe, expect, it } from 'vitest';
import type { AppUninstallInspection, LargeFileItem } from '../lib/models/types';
import {
  LARGE_FILE_DEFAULT_THRESHOLD_BYTES,
  LARGE_FILE_MIN_BYTES,
  clampLargeFileThreshold,
  defaultRelatedIds,
  largeFileKindLabel,
  selectedAppTrashBytes,
  selectedLargeFileBytes,
} from '../lib/utils/storageManagement';

const MIB = 1024 * 1024;

describe('storage management helpers', () => {
  it('enforces the backend large-file minimum threshold', () => {
    expect(clampLargeFileThreshold(10 * MIB)).toBe(LARGE_FILE_MIN_BYTES);
    expect(clampLargeFileThreshold(750 * MIB)).toBe(750 * MIB);
    expect(clampLargeFileThreshold(Number.NaN)).toBe(LARGE_FILE_DEFAULT_THRESHOLD_BYTES);
  });

  it('counts only explicitly selected large files by allocated size', () => {
    const items: LargeFileItem[] = [
      {
        id: 'one',
        name: 'one.mov',
        display_parent: '/tmp',
        logical_size: 100 * MIB,
        allocated_size: 80 * MIB,
        modified_at: null,
        kind: 'video',
        extension: 'mov',
      },
      {
        id: 'two',
        name: 'two.zip',
        display_parent: '/tmp',
        logical_size: 200 * MIB,
        allocated_size: 160 * MIB,
        modified_at: null,
        kind: 'archive',
        extension: 'zip',
      },
    ];

    expect(selectedLargeFileBytes(items, ['two'])).toBe(160 * MIB);
    expect(selectedLargeFileBytes(items, ['missing'])).toBe(0);
  });

  it('auto-selects only high-confidence related app data', () => {
    const inspection: AppUninstallInspection = {
      inspection_id: 'inspection-1',
      app: {
        id: 'app-1',
        name: 'Example',
        bundle_id: 'com.example.app',
        version: '1.0',
        display_path: '/Applications/Example.app',
        executable_name: 'Example',
        logical_size: 300 * MIB,
        allocated_size: 310 * MIB,
        modified_at: null,
        install_source: 'application_bundle',
        is_running: false,
        is_system_protected: false,
      },
      related_items: [
        {
          id: 'high',
          name: 'com.example.app',
          display_path: '/Users/test/Library/Caches/com.example.app',
          kind: 'cache',
          confidence: 'high',
          evidence: 'Exact CFBundleIdentifier match',
          logical_size: 20 * MIB,
          allocated_size: 24 * MIB,
          selected_by_default: true,
        },
        {
          id: 'medium',
          name: 'Example',
          display_path: '/Users/test/Library/Logs/Example',
          kind: 'log',
          confidence: 'medium',
          evidence: 'Exact application display-name match',
          logical_size: 5 * MIB,
          allocated_size: 6 * MIB,
          selected_by_default: false,
        },
        {
          id: 'shared',
          name: 'group.com.example.app',
          display_path: '/Users/test/Library/Group Containers/group.com.example.app',
          kind: 'group_container',
          confidence: 'shared',
          evidence: 'Shared container',
          logical_size: 40 * MIB,
          allocated_size: 44 * MIB,
          selected_by_default: false,
        },
      ],
      incomplete: false,
      warnings: [],
    };

    expect(defaultRelatedIds(inspection)).toEqual(['high']);
    expect(selectedAppTrashBytes(inspection, ['high'])).toBe(334 * MIB);
    expect(selectedAppTrashBytes(inspection, ['high', 'medium'])).toBe(340 * MIB);
  });

  it('uses stable human-readable labels for file categories', () => {
    expect(largeFileKindLabel('ai_model')).toBe('AI Model');
    expect(largeFileKindLabel('developer_artifact')).toBe('Developer Artifact');
    expect(largeFileKindLabel('other')).toBe('Other');
  });
});
