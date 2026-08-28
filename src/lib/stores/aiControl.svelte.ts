import type {
  AiControlCenterSnapshot,
  AiControlPreferences,
  DashboardRoute,
  RecommendationPreview,
} from '../models/types';
import {
  tauriConsumeAiRecommendationPreview,
  tauriDismissAiSafetyFinding,
  tauriGetAiControlCenter,
  tauriGetAiControlGitDiff,
  tauriPreviewAiRecommendation,
  tauriRunAiSafetyScan,
  tauriSaveAiControlPreferences,
} from '../utils/tauri';

export class AiControlStore {
  snapshot = $state<AiControlCenterSnapshot | null>(null);
  preview = $state<RecommendationPreview | null>(null);
  gitDiff = $state<string | null>(null);
  isLoading = $state(false);
  isScanning = $state(false);
  error = $state<string | null>(null);

  async refresh(force = false) {
    if (this.isLoading) return;
    this.isLoading = true;
    try {
      this.snapshot = await tauriGetAiControlCenter(force);
      this.error = null;
    } catch (error) {
      this.error = error instanceof Error ? error.message : 'AI Control Center is unavailable.';
    } finally {
      this.isLoading = false;
    }
  }

  async savePreferences(preferences: AiControlPreferences) {
    await tauriSaveAiControlPreferences(preferences);
    await this.refresh(true);
  }

  async scanSafety() {
    if (this.isScanning) return;
    this.isScanning = true;
    try {
      const safety = await tauriRunAiSafetyScan();
      if (this.snapshot) {
        this.snapshot = {
          ...this.snapshot,
          safety,
          quick_summary: {
            ...this.snapshot.quick_summary,
            safety_findings: safety.findings.filter((f) => !f.dismissed).length,
          },
        };
      }
      this.error = null;
    } catch (error) {
      this.error = error instanceof Error ? error.message : 'Safety inspection failed.';
    } finally {
      this.isScanning = false;
    }
  }

  async dismissFinding(findingId: string) {
    await tauriDismissAiSafetyFinding(findingId);
    if (this.snapshot) {
      const updatedFindings = this.snapshot.safety.findings.map((finding) =>
        finding.id === findingId ? { ...finding, dismissed: true } : finding
      );
      this.snapshot = {
        ...this.snapshot,
        safety: {
          ...this.snapshot.safety,
          findings: updatedFindings,
        },
        quick_summary: {
          ...this.snapshot.quick_summary,
          safety_findings: updatedFindings.filter((f) => !f.dismissed).length,
        },
      };
    }
  }

  async createPreview(recommendationId: string) {
    this.preview = await tauriPreviewAiRecommendation(recommendationId);
  }

  async consumePreview(): Promise<DashboardRoute | null> {
    if (!this.preview) return null;
    const consumed = await tauriConsumeAiRecommendationPreview(this.preview.id);
    this.preview = null;
    return consumed.destination;
  }

  async loadGitDiff(projectId: string) {
    this.gitDiff = await tauriGetAiControlGitDiff(projectId);
  }

  clearGitDiff() {
    this.gitDiff = null;
  }
}

export const aiControlStore = new AiControlStore();
