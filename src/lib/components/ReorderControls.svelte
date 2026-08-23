<script lang="ts">
  import { ArrowDown, ArrowUp } from 'lucide-svelte';
  import Button from './Button.svelte';

  interface Props {
    label: string;
    index: number;
    count: number;
    onMove: (direction: -1 | 1) => void | Promise<void>;
  }

  let { label, index, count, onMove }: Props = $props();
</script>

<div class="flex shrink-0 items-center gap-0.5" aria-label={`Reorder ${label}`}>
  <Button
    variant="ghost"
    size="icon"
    class="h-6 w-6"
    disabled={index <= 0}
    onclick={() => void onMove(-1)}
    ariaLabel={`Move ${label} up`}
    title={`Move ${label} up`}
  >
    <ArrowUp size={12} />
  </Button>
  <Button
    variant="ghost"
    size="icon"
    class="h-6 w-6"
    disabled={index === -1 || index >= count - 1}
    onclick={() => void onMove(1)}
    ariaLabel={`Move ${label} down`}
    title={`Move ${label} down`}
  >
    <ArrowDown size={12} />
  </Button>
</div>
