/**
 * Tauri injects the per-build CSP nonce into the static styles in index.html.
 * Reusing that nonce lets Ant Design's CSS-in-JS styles pass the same policy
 * without weakening style-src with unsafe-inline or disabling Tauri CSP edits.
 */
export function getCspNonce(): string | undefined {
  if (typeof document === 'undefined') return undefined;

  const style = document.querySelector<HTMLStyleElement>('style[nonce]');
  return style?.nonce || undefined;
}
