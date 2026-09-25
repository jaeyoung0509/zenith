<script lang="ts">
  import { AlertCircle, AlertTriangle, CheckCircle2, Info, X } from "@lucide/svelte";

  interface Props {
    variant?: "info" | "warning" | "error" | "destructive" | "success";
    title?: string;
    message: string;
    onDismiss?: () => void;
    actionLabel?: string;
    onAction?: () => void;
    class?: string;
  }

  let {
    variant = "info",
    title,
    message,
    onDismiss,
    actionLabel,
    onAction,
    class: className = "",
  }: Props = $props();

  const variantStyles = {
    info: "bg-secondary border-border text-foreground",
    warning: "bg-warning/10 border-warning/30 text-warning",
    error: "bg-destructive/10 border-destructive/30 text-destructive",
    destructive: "bg-destructive/10 border-destructive/30 text-destructive",
    success: "bg-success/10 border-success/30 text-success",
  };

  const icons = {
    info: Info,
    warning: AlertTriangle,
    error: AlertCircle,
    destructive: AlertCircle,
    success: CheckCircle2,
  };
  let Icon = $derived(icons[variant]);
</script>

<div
  role={variant === "error" || variant === "destructive" ? "alert" : "status"}
  class="flex items-start justify-between gap-3 rounded-xl border p-3 text-meta leading-relaxed {variantStyles[variant]} {className}"
>
  <div class="flex items-start gap-2.5 min-w-0">
    <Icon size={16} class="shrink-0 mt-0.5" />
    <div class="min-w-0 space-y-0.5">
      {#if title}
        <div class="font-semibold">{title}</div>
      {/if}
      <div class="text-meta leading-normal break-words">{message}</div>
    </div>
  </div>

  <div class="flex items-center gap-2 shrink-0">
    {#if actionLabel && onAction}
      <button
        type="button"
        onclick={onAction}
        class="rounded-sm text-meta font-semibold underline underline-offset-2 transition-ui hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {actionLabel}
      </button>
    {/if}
    {#if onDismiss}
      <button
        type="button"
        onclick={onDismiss}
        aria-label="Dismiss notice"
        title="Dismiss notice"
        class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-ui hover:bg-foreground/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <X size={14} />
      </button>
    {/if}
  </div>
</div>
