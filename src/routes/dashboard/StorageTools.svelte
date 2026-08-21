<script lang="ts">
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
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
    <Card class="p-4 bg-card/60 border-border/70">
      <div class="flex items-center gap-3">
        <div class="h-9 w-9 rounded-lg bg-secondary/70 border border-border/60 flex items-center justify-center shrink-0">
          <FileSearch size={17} class="text-muted-foreground" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="text-xs font-semibold">Large Files</div>
          <p class="text-[10px] text-muted-foreground mt-0.5">
            Find files taking significant disk space
          </p>
        </div>
        <Button variant="ghost" size="icon" onclick={onOpenLargeFiles} ariaLabel="Open Large Files">
          <ChevronRight size={15} />
        </Button>
      </div>
    </Card>

    <Card class="p-4 bg-card/60 border-border/70">
      <div class="flex items-center gap-3">
        <div class="h-9 w-9 rounded-lg bg-secondary/70 border border-border/60 flex items-center justify-center shrink-0">
          <AppWindow size={17} class="text-muted-foreground" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="text-xs font-semibold">Applications</div>
          <p class="text-[10px] text-muted-foreground mt-0.5">
            Remove apps and review related files
          </p>
        </div>
        <Button variant="ghost" size="icon" onclick={onOpenApplications} ariaLabel="Open Applications">
          <ChevronRight size={15} />
        </Button>
      </div>
    </Card>
  </div>
</div>
