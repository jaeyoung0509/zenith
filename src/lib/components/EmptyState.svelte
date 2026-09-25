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

<div class="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border bg-secondary p-8 text-center {className}">
  {#if Icon}
    <div class="h-10 w-10 rounded-full border border-border bg-card flex items-center justify-center text-muted-foreground">
      <Icon size={20} />
    </div>
  {/if}
  <div class="max-w-sm space-y-1">
    <h3 class="text-body font-semibold text-foreground">{title}</h3>
    {#if description}
      <p class="text-meta text-muted-foreground">{description}</p>
    {/if}
  </div>
  {#if actionLabel && onAction}
    <Button variant="outline" size="sm" onclick={onAction}>
      {actionLabel}
    </Button>
  {/if}
</div>
