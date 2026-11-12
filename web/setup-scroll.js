import {focusable, focusNode} from './navigation.js';

// 桌面 CSP 禁止更新 HTML style 属性，CSSOM 属性赋值可正常生效。
// 必须在恢复滚动位置和测量视口之前同步，保证复用的占位行立即采用新高度。
export function syncSetupSpacers(scroller) {
  for (const cell of scroller?.querySelectorAll('[data-setup-spacer-rows]') || []) {
    const rows = Number(cell.dataset.setupSpacerRows);
    if (Number.isSafeInteger(rows) && rows >= 0) {
      cell.style.height = `calc(var(--setup-row-height) * ${rows})`;
    }
  }
}

// Geometry and focus only. Drafts and the full batch queue belong to Rust.
export function installSetupScroll(update) {
  let scroller = null;
  let frame = 0;
  const resize = new ResizeObserver(schedule);

  function schedule() {
    if (!frame) frame = requestAnimationFrame(sync);
  }

  function sync() {
    frame = 0;
    const next = document.getElementById('setup-table-scroll');
    if (next !== scroller) {
      resize.disconnect();
      scroller?.removeEventListener('scroll', schedule);
      scroller = next;
      if (scroller) {
        scroller.addEventListener('scroll', schedule, {passive: true});
        resize.observe(scroller);
        resize.observe(scroller.querySelector('table'));
      }
    }
    if (!scroller) return;
    const body = scroller.querySelector('tbody');
    const row = body.querySelector('[data-setup-index]');
    if (!row) return;
    const height = row.getBoundingClientRect().height;
    if (!height) return;
    const total = Number(body.dataset.setupTotal);
    const first = Math.floor(scroller.scrollTop / height);
    const visible = Math.ceil(scroller.clientHeight / height);
    const anchor = document.querySelector('.choice-popup')?.dataset.choiceAnchor;
    const pinned = (anchor ? document.getElementById(anchor) : document.activeElement)
      ?.closest('[data-setup-game]')?.dataset.setupGame || '';
    let start = Number(body.dataset.setupStart);
    let count = Number(body.dataset.setupCount);
    // Keep a few rows ahead of keyboard/controller focus and avoid a Rust paint per pixel.
    if ((first < start + 4 && start > 0) ||
        (first + visible > start + count - 4 && start + count < total)) {
      count = Math.max(16, Math.min(80, visible + 16));
      start = Math.min(Math.max(0, first - 8), Math.max(0, total - count));
    }
    if (start !== Number(body.dataset.setupStart) ||
        count !== Number(body.dataset.setupCount) || pinned !== body.dataset.setupPinned) {
      update(start, count, pinned);
    }
  }

  document.addEventListener('focusin', schedule);
  document.addEventListener('keydown', event => {
    if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) return;
    const row = event.target.closest('[data-setup-index]');
    if (!row) return;
    const controls = focusable(row);
    if (event.target !== (event.shiftKey ? controls[0] : controls.at(-1))) return;
    const index = Number(row.dataset.setupIndex) + (event.shiftKey ? -1 : 1);
    const body = row.parentElement;
    if (index < 0 || index >= Number(body.dataset.setupTotal) ||
        body.querySelector(`[data-setup-index="${index}"]`)) return;
    // A row pinned while the user wheels elsewhere still tabs to its logical neighbour.
    event.preventDefault();
    event.stopImmediatePropagation();
    const scroll = row.closest('.setup-table-scroll');
    scroll.scrollTop = index * row.getBoundingClientRect().height;
    update(Math.max(0, index - 8), 32, row.dataset.setupGame);
    requestAnimationFrame(() => {
      const adjacent = document.querySelector(`[data-setup-index="${index}"]`);
      const nodes = focusable(adjacent);
      focusNode(event.shiftKey ? nodes.at(-1) : nodes[0]);
    });
  }, true);
  return schedule;
}
