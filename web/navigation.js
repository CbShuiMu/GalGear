// Shared focus boundaries for keyboard, controller and modal rendering.
export const focusSelector = 'button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex="0"]';
export function topLayer() {
  for (const selector of ['.startup-dialog', '.update-dialog', '.controller-keyboard', '.shortcut-dialog', '.choice-popup', '.tag-remove-dialog', '.snapshot-delete-dialog', '.setup-dialog', '.library-dialog', '.ai-summary-dialog', '.restore-dialog', '.archive-dialog', '.editor-dialog', '.game-context-menu', '.detail-panel']) {
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
  const overlay = document.querySelector('.startup-overlay') || document.querySelector('.update-overlay') || document.querySelector('.snapshot-delete-dialog')?.closest('.editor-overlay') || document.querySelector('.setup-overlay') || document.querySelector('.library-overlay') ||
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

let updateReturnFocus = null;
// 启动检查可能在其他弹窗中完成；更新提示关闭后回到原控件。
export function syncUpdateDialogFocus(previousFocus) {
  const dialog = document.querySelector('.update-dialog');
  if (dialog) {
    const opening = !updateReturnFocus;
    updateReturnFocus ||= {node: previousFocus, id: previousFocus?.id,
      action: previousFocus?.dataset.action, value: previousFocus?.dataset.value};
    if (opening || !dialog.contains(document.activeElement)) {
      dialog.querySelector('.dialog-actions [data-action="update-cancel"]:not(:disabled)')?.focus({preventScroll:true});
    }
  } else if (updateReturnFocus) {
    const origin = updateReturnFocus;
    updateReturnFocus = null;
    const target = (origin.node?.isConnected && origin.node) ||
      (origin.id && document.getElementById(origin.id)) ||
      (origin.action && [...document.querySelectorAll('[data-action]')].find(node =>
        node.dataset.action === origin.action && node.dataset.value === origin.value));
    const candidates = focusable();
    (candidates.includes(target) ? target : candidates[0])?.focus({preventScroll:true});
  }
}
