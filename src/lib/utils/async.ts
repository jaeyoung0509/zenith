/**
 * Resolves with the wrapped promise's result, but never faster than
 * `minimumMs`, so brief operations do not flicker loading states on and off.
 */
export async function withMinimumDuration<T>(
  promise: Promise<T>,
  minimumMs: number,
): Promise<T> {
  const startedAt = Date.now();
  const result = await promise;
  const elapsed = Date.now() - startedAt;
  if (elapsed < minimumMs) {
    await new Promise((resolve) => setTimeout(resolve, minimumMs - elapsed));
  }
  return result;
}
