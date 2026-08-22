<script lang="ts">
  import { AppWindow, ChevronRight, FileSearch } from 'lucide-svelte';

  interface Props {
    onOpenLargeFiles: () => void;
    onOpenApplications: () => void;
  }

  let { onOpenLargeFiles, onOpenApplications }: Props = $props();
</script>

<div class="space-y-3">
  <div>
    <h2 class="text-sm font-semibold text-foreground tracking-tight">Storage Tools</h2>
    <p class="text-[11px] text-muted-foreground mt-0.5">
      User-reviewed workflows stay separate from automatic cache cleanup.
    </p>
  </div>

  <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
    <button
      type="button"
      onclick={onOpenLargeFiles}
      aria-label="Open Large Files"
      class="group w-full rounded-xl border border-border/70 bg-card/60 p-4 text-left text-card-foreground shadow-sm backdrop-blur-sm transition-colors hover:border-border hover:bg-secondary/30 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
    >
      <span class="flex items-center gap-3">
        <span class="h-9 w-9 rounded-lg bg-secondary/70 border border-border/60 flex items-center justify-center shrink-0">
          <FileSearch size={17} class="text-muted-foreground" />
        </span>
        <span class="min-w-0 flex-1">
          <span class="block text-xs font-semibold">Large Files</span>
          <span class="block text-[10px] text-muted-foreground mt-0.5">
            Find files taking significant disk space
          </span>
        </span>
        <span aria-hidden="true" class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5">
          <ChevronRight size={15} />
        </span>
      </span>
    </button>

    <button
      type="button"
      onclick={onOpenApplications}
      aria-label="Open Applications"
      class="group w-full rounded-xl border border-border/70 bg-card/60 p-4 text-left text-card-foreground shadow-sm backdrop-blur-sm transition-colors hover:border-border hover:bg-secondary/30 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
    >
      <span class="flex items-center gap-3">
        <span class="h-9 w-9 rounded-lg bg-secondary/70 border border-border/60 flex items-center justify-center shrink-0">
          <AppWindow size={17} class="text-muted-foreground" />
        </span>
        <span class="min-w-0 flex-1">
          <span class="block text-xs font-semibold">Applications</span>
          <span class="block text-[10px] text-muted-foreground mt-0.5">
            Remove apps and review related files
          </span>
        </span>
        <span aria-hidden="true" class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5">
          <ChevronRight size={15} />
        </span>
      </span>
    </button>
  </div>
</div>
