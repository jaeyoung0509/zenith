<script lang="ts">
  import Button from "./Button.svelte";

  interface Props {
    icon?: any;
    title: string;
    description?: string;
    actionLabel?: string;
    onAction?: () => void;
    class?: string;
  }

  let {
    icon: Icon,
    title,
    description,
    actionLabel,
    onAction,
    class: className = "",
  }: Props = $props();
</script>

<div class="p-8 text-center rounded-xl border border-dashed border-border/70 bg-card/30 flex flex-col items-center justify-center space-y-3 {className}">
  {#if Icon}
    <div class="h-10 w-10 rounded-full bg-secondary/80 flex items-center justify-center text-muted-foreground">
      <Icon size={20} />
    </div>
  {/if}
  <div class="space-y-1 max-w-sm">
    <h3 class="text-xs font-semibold text-foreground">{title}</h3>
    {#if description}
      <p class="text-caption text-muted-foreground">{description}</p>
    {/if}
  </div>
  {#if actionLabel && onAction}
    <Button variant="outline" size="xs" onclick={onAction} class="mt-2">
      {actionLabel}
    </Button>
  {/if}
</div>
