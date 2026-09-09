<script lang="ts">
  import type { Snippet } from "svelte";
  import Badge from "./Badge.svelte";

  interface Props {
    title: string;
    subtitle?: string;
    icon?: any;
    badge?:
      | string
      | {
          label: string;
          variant?: "default" | "secondary" | "outline" | "success" | "warning" | "ai" | "destructive";
        }
      | Snippet;
    class?: string;
    actions?: Snippet;
  }

  let {
    title,
    subtitle,
    icon: Icon,
    badge,
    class: className = "",
    actions,
  }: Props = $props();
</script>

<header class="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-3 border-b border-border/60 {className}">
  <div class="flex items-center gap-3 min-w-0">
    {#if Icon}
      <div class="h-9 w-9 rounded-lg bg-secondary/80 border border-border/60 text-foreground flex items-center justify-center shrink-0 shadow-xs">
        <Icon size={18} />
      </div>
    {/if}
    <div class="min-w-0">
      <div class="flex items-center gap-2">
        <h1 class="text-base font-semibold text-foreground tracking-tight truncate">{title}</h1>
        {#if typeof badge === "string"}
          <Badge variant="outline">{badge}</Badge>
        {:else if typeof badge === "function"}
          {@render (badge as Snippet)()}
        {:else if badge && "label" in badge}
          <Badge variant={badge.variant ?? "outline"}>{badge.label}</Badge>
        {/if}
      </div>
      {#if subtitle}
        <p class="text-xs text-muted-foreground mt-0.5 truncate">{subtitle}</p>
      {/if}
    </div>
  </div>

  {#if actions}
    <div class="flex items-center gap-2 shrink-0">
      {@render actions()}
    </div>
  {/if}
</header>
