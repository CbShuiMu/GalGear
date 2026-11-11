// DOM geometry and input only. Rust owns the selected size and persistence.
export function libraryCardLayout(width, viewport, size, gap = 20) {
  const fit = card => Math.max(1, Math.floor((width + gap + .5) / (card + gap)));
  const normal = Math.max(227, Math.min(viewport * .18, 312));
  const normalColumns = fit(normal);
  if (size !== 'small' && size !== 'tiny') return {columns: normalColumns, width: Math.min(normal, width)};
  const small = Math.max(177, Math.min(viewport * .10, 205));
  const smallColumns = Math.max(normalColumns + 1, fit(small));
  const tiny = Math.max(145, Math.min(viewport * .08, 166));
  const columns = size === 'small' ? smallColumns : Math.max(smallColumns + 1, fit(tiny));
  // Integer column steps prevent two adjacent sizes from showing the same count.
  return {columns, width: Math.max(1, Math.min(size === 'small' ? small : tiny, (width - (columns - 1) * gap) / columns))};
}

export function syncLibrarySize(page, grid) {
  if (!grid) return;
  const gap = parseFloat(getComputedStyle(grid).columnGap) || 0;
  const layout = libraryCardLayout(grid.getBoundingClientRect().width, window.innerWidth, page.dataset.cardSize, gap);
  const columns = String(layout.columns);
  if (grid.style.getPropertyValue('--library-columns') !== columns) grid.style.setProperty('--library-columns', columns);
  const value = `${layout.width}px`;
  if (grid.style.getPropertyValue('--card-width') !== value) grid.style.setProperty('--card-width', value);
  return layout;
}

export function installLibrarySize({step, isDragging}) {
  let lastWheel = -Infinity;
  function allowed(event) {
    const page = document.querySelector('.library-page');
    return event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.isComposing
      && page && !page.closest('[inert]') && !isDragging()
      && !document.querySelector('[aria-modal="true"],.choice-popup,.game-context-menu,.detail-overlay')
      && !event.target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"])');
  }
  document.addEventListener('keydown', event => {
    if (!['ArrowUp', 'ArrowDown'].includes(event.key) || !allowed(event)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (!event.repeat) step(event.key === 'ArrowUp' ? 'larger' : 'smaller');
  }, true);
  document.addEventListener('wheel', event => {
    if (!event.deltaY || !allowed(event)) return;
    const page = document.querySelector('.library-page');
    // Shrinking cards can leave the pointer below the page, inside its scroll pane.
    const surface = page.closest('.page-scroll') || page;
    if (!surface.contains(event.target)) return;
    event.preventDefault();
    // Touchpads emit many wheel events per gesture; avoid skipping both size steps.
    const now = performance.now();
    if (now - lastWheel >= 200) { lastWheel = now; step(event.deltaY < 0 ? 'larger' : 'smaller'); }
  }, {passive: false});
}
