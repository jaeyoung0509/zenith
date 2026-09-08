// Run in the local Vite preview with Memory selected:
// agent-browser --session zenith126 eval --stdin < scripts/verify_numeric_layout.js
// Dev-only verification: mutates browser mocks, never invokes a native action.
(async () => {
  if (!['localhost', '127.0.0.1'].includes(location.hostname) || window.__TAURI_INTERNALS__) {
    throw new Error('Use the local browser preview, never the native app');
  }
  const moduleUrl = performance.getEntriesByType('resource')
    .map((entry) => entry.name).filter((name) => name.includes('/src/lib/stores/memory.svelte.ts')).at(-1);
  const { memoryStore } = await import(moduleUrl);
  memoryStore.stopPolling();
  const original = structuredClone(JSON.parse(JSON.stringify(memoryStore.memory)));
  if (!original) throw new Error('Open Memory and wait for the fixture');
  const frame = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  // Sidebar expansion is an intentional transition, not numeric layout jitter.
  await Promise.all(document.getAnimations()
    .filter((animation) => animation.effect?.getTiming().iterations !== Infinity)
    .map((animation) => animation.finished.catch(() => undefined)));
  const results = [];
  // Isolated reproduction of the previous Physical Memory text block at the
  // baseline card's 182px content width; never used as application UI.
  const legacy = document.createElement('div');
  legacy.className = 'text-2xl font-bold font-mono';
  legacy.style.cssText = 'position:fixed;left:-10000px;width:182px';
  document.body.append(legacy);
  const previousHeights = ['9 GB / 16 GB', '10 GB / 16 GB', '100 GB / 128 GB'].map((text) => {
    legacy.textContent = text;
    return { text, height: legacy.getBoundingClientRect().height };
  });
  legacy.remove();
  try {
    for (const bytes of [9.9 * 1024 ** 2, 10 * 1024 ** 2, 100 * 1024 ** 2, 999.9 * 1024 ** 2, 1024 ** 3, 100 * 1024 ** 3, 1023.9 * 1024 ** 3]) {
      memoryStore.memory = {
        ...original, used_bytes: bytes, total_bytes: 1024 ** 4,
        compressed_bytes: bytes, swap_used_bytes: bytes, swap_total_bytes: 1024 ** 4,
        top_processes: original.top_processes.map((process, index) => ({
          ...process, memory_bytes: bytes, process_count: 100,
          name: index % 2 ? `한국어 개발 애플리케이션 긴 이름 ${index}` : process.name,
        })),
      };
      await frame();
      const values = [...document.querySelectorAll('[data-byte-value]')];
      const rows = [...document.querySelectorAll('[data-process-row]')];
      if (values.length < 3 || rows.length === 0) throw new Error('Missing rendered metric fixtures; reload after HMR');
      const rect = (element) => {
        const bounds = element.getBoundingClientRect();
        return [bounds.x, bounds.y, bounds.width, bounds.height].map((n) => Math.round(n * 100) / 100);
      };
      const overflow = values.filter((element) => {
        const bounds = element.getBoundingClientRect();
        const parent = element.closest('.memory-gauges > div, [data-process-row]')?.getBoundingClientRect();
        return element.scrollWidth > element.clientWidth + 1 || bounds.right > innerWidth + 1 || (parent && bounds.right > parent.right + 1);
      });
      if (overflow.length) throw new Error(`Numeric overflow at ${bytes}`);
      results.push({ bytes, gauges: [...document.querySelector('.memory-gauges').children].map(rect),
        actions: rows.map((row) => row.querySelector('button')).filter(Boolean).map(rect),
        rowMetrics: rows.map((row) => rect(row.querySelector('[data-byte-value]'))) });
    }
    const baseline = JSON.stringify({ gauges: results[0].gauges, actions: results[0].actions, rowMetrics: results[0].rowMetrics });
    for (const result of results) {
      if (JSON.stringify({ gauges: result.gauges, actions: result.actions, rowMetrics: result.rowMetrics }) !== baseline) {
        throw new Error(`Layout moved at ${result.bytes}: ${JSON.stringify(results)}`);
      }
    }
    return { viewport: [innerWidth, innerHeight], cases: results.length, previousHeights, numericOverflow: 0, layoutMovementPx: 0, baseline: results[0] };
  } finally {
    memoryStore.memory = original;
  }
})()
