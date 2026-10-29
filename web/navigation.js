// Shared focus boundaries for keyboard, controller and modal rendering.
export const focusSelector = 'button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex="0"]';
export function topLayer() {
  for (const selector of ['.controller-keyboard', '.choice-popup', '.setup-dialog', '.library-dialog', '.ai-summary-dialog', '.restore-dialog', '.editor-dialog', '.game-context-menu', '.detail-panel']) {
    const layer = document.querySelector(selector);
    if (layer && !layer.closest('[inert]')) return layer;
  }
  return document.querySelector('.app-shell');
}
export function focusable(root = topLayer()) {
  return [...(root?.querySelectorAll(focusSelector) || [])].filter(node =>
    !node.closest('[inert]') && node.getClientRects().length && getComputedStyle(node).visibility !== 'hidden');
}
export function focusNode(node) {
  if (!node) return;
  node.focus({preventScroll:true});
  if (!node.closest('[data-focus-scroll="managed"]')) {
    node.scrollIntoView({block:'nearest', inline:'nearest', behavior:'instant'});
  }
}
export function syncModalBoundary() {
  const overlay = document.querySelector('.setup-overlay') || document.querySelector('.library-overlay') ||
    document.querySelector('.ai-summary-dialog')?.closest('.editor-overlay') ||
    document.querySelector('.restore-dialog')?.closest('.editor-overlay') || document.querySelector('.editor-overlay');
  document.querySelectorAll('.sidebar,.page-scroll,.detail-overlay,.editor-overlay,.library-overlay').forEach(node => {
    const covered = overlay ? node !== overlay && !node.contains(overlay) : node.matches('.page-scroll') && !!document.querySelector('.detail-overlay');
    node.inert = !!covered;
  });
  // Inert blocks input, while overflow containment also prevents scroll chaining.
  document.querySelectorAll('.page-scroll,.detail-scroll').forEach(node => {
    node.classList.toggle('modal-scroll-locked', !!node.closest('[inert]'));
  });
}
