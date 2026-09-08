<script lang="ts">
  import { formatBytes } from '../utils/format';

  interface Props {
    bytes: number;
    class?: string;
  }

  let { bytes, class: className = '' }: Props = $props();
</script>

<span data-byte-value class="inline-block whitespace-nowrap font-mono tabular-nums {className}">{formatBytes(bytes)}</span>
