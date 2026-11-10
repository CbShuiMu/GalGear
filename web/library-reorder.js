// DOM-only long-press gesture. Rust validates and persists the resulting move.
export function installLibraryReorder({ idsFor, start, finish, commit }) {
  let hold = null;
  let drag = null;
  let frame = 0;
  let settling = false;
  const layoutAnimations = new WeakMap();
  const easing = 'cubic-bezier(.4,0,.2,1)';
  const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Measure the visible position before cancelling an interrupted transition, then
  // measure the new layout without transforms. Hit testing uses these stable slots.
  function transitionLayout(cards, change) {
    const positions = new Map(cards.map(card => [card, card.getBoundingClientRect()]));
    cards.forEach(card => layoutAnimations.get(card)?.cancel());
    change();
    const targets = new Map(cards.map(card => [card, card.getBoundingClientRect()]));
    if (!reducedMotion()) {
      for (const card of cards) {
        const old = positions.get(card), next = targets.get(card);
        if (old.x === next.x && old.y === next.y) continue;
        const animation = card.animate([
          { transform: `translate(${old.x - next.x}px,${old.y - next.y}px)` },
          { transform: 'none' }
        ], { duration: 280, easing });
        layoutAnimations.set(card, animation);
      }
    }
    return targets;
  }

  function arrange(before) {
    drag.slots = transitionLayout(drag.cards, () => {
      if (before === undefined) drag.gap.remove();
      else drag.grid.insertBefore(drag.gap, before);
    });
    drag.slotScroll = drag.pane.scrollTop;
  }

  function move() {
    if (!drag) return;
    const { x, y, grid, preview, pane } = drag;
    preview.style.transform = `translate(${x + 14}px,${y + 12}px)`;
    const viewport = pane.getBoundingClientRect();
    if (y > viewport.bottom - 36) pane.scrollTop += 12;
    else if (y < viewport.top + 36) pane.scrollTop -= 12;
    const bounds = grid.getBoundingClientRect();
    const inside = x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= Math.min(bounds.bottom + 40, viewport.bottom);
    const gap = drag.gap.getBoundingClientRect();
    if (!inside) {
      if (drag.valid) arrange(undefined);
      drag.valid = false;
    } else if (!(drag.valid && x >= gap.left && x <= gap.right && y >= gap.top && y <= gap.bottom)) {
      const before = drag.cards.find(card => {
        const rect = drag.slots.get(card);
        const scroll = pane.scrollTop - drag.slotScroll;
        return y < rect.top - scroll || y <= rect.bottom - scroll && x < rect.left + rect.width / 2;
      }) || null;
      if (!drag.valid || before !== drag.before) arrange(before);
      drag.before = before;
      drag.valid = true;
    }
    frame = requestAnimationFrame(move);
  }

  function begin() {
    const pending = hold;
    hold = null;
    if (!pending || !pending.card.isConnected) return;
    const ids = JSON.parse(idsFor(pending.card.dataset.gameId) || '[]');
    if (!ids.length) return;
    start(JSON.stringify(ids));
    const all = [...pending.grid.querySelectorAll('.game-card')];
    const moving = all.filter(card => ids.includes(card.dataset.gameId));
    const box = pending.card.getBoundingClientRect();
    const preview = document.createElement('div');
    preview.className = 'reorder-stack';
    preview.setAttribute('aria-hidden', 'true');
    preview.inert = true;
    const previewWidth = Math.min(box.width, 150);
    preview.style.width = `${previewWidth}px`;
    preview.style.height = `${box.height * previewWidth / box.width}px`;
    preview.style.transform = `translate(${pending.x + 14}px,${pending.y + 12}px)`;
    const positions = new Map(all.map(card => [card, card.getBoundingClientRect()]));
    moving.forEach((card, index) => {
      const copy = card.cloneNode(true);
      copy.removeAttribute('id');
      copy.classList.remove('is-selected', 'is-selectable');
      copy.querySelector('.selection-check')?.remove();
      copy.querySelectorAll('[id]').forEach(node => node.removeAttribute('id'));
      copy.style.setProperty('--stack-index', index);
      copy.style.zIndex = moving.length - index;
      preview.append(copy);
      if (!reducedMotion()) {
        const original = positions.get(card);
        copy.animate([
          { transform: `translate(${original.x - pending.x - 14}px,${original.y - pending.y - 12}px) scale(${original.width / Math.min(box.width, 150)})`, transformOrigin: 'top left' },
          { transform: `translateX(${index * 12}px) scale(1)`, transformOrigin: 'top left' }
        ], { duration: 320, easing });
      }
    });
    const gap = document.createElement('div');
    gap.className = 'reorder-gap';
    gap.style.height = `${box.height}px`;
    (pending.grid.closest('.app-shell') || document.body).append(preview);
    const cards = all.filter(card => !moving.includes(card));
    const slots = transitionLayout(cards, () => moving.forEach(card => { card.hidden = true; }));
    const pane = document.getElementById('page-scroll');
    drag = { ...pending, ids, all, moving, cards, preview, gap, valid: false, pane, slots, slotScroll: pane.scrollTop };
    pending.grid.setPointerCapture(pending.pointerId);
    move();
  }

  async function land(current) {
    settling = true;
    const wasInert = current.grid.inert;
    current.grid.inert = true;
    const copies = [...current.preview.children];
    const origins = copies.map(copy => copy.getBoundingClientRect());
    const animations = [];
    try {
      // Hidden real cards reserve all destination slots before the stack separates.
      transitionLayout(current.cards, () => {
        current.gap.remove();
        for (const card of current.moving) {
          current.grid.insertBefore(card, current.before);
          card.classList.add('reorder-landing');
          card.hidden = false;
        }
      });
      copies.forEach((copy, index) => {
        const origin = origins[index];
        const target = current.moving[index].getBoundingClientRect();
        copy.getAnimations().forEach(animation => animation.cancel());
        copy.classList.add('reorder-flight');
        Object.assign(copy.style, {
          left: `${origin.x}px`, top: `${origin.y}px`, right: 'auto', bottom: 'auto',
          width: `${origin.width}px`, height: `${origin.height}px`, transform: 'none',
          zIndex: String(140 + copies.length - index)
        });
        current.preview.parentElement.append(copy);
        if (!reducedMotion()) {
          animations.push(copy.animate([
            { transform: 'translate(0,0) scale(1)' },
            { transform: `translate(${target.x - origin.x}px,${target.y - origin.y}px) scale(${target.width / origin.width},${target.height / origin.height})` }
          ], { duration: 420, easing, fill: 'forwards' }));
        } else {
          Object.assign(copy.style, {
            left: `${target.x}px`, top: `${target.y}px`,
            width: `${target.width}px`, height: `${target.height}px`
          });
        }
      });
      current.preview.remove();
      // Keep the last frame until Rust has returned the authoritative order. A
      // failed save then restores that order, without displaying a false success.
      await Promise.allSettled([
        commit(current.ids, current.before?.dataset.gameId || null),
        ...animations.map(animation => animation.finished)
      ]);
    } finally {
      animations.forEach(animation => animation.cancel());
      copies.forEach(copy => copy.remove());
      current.preview.remove();
      current.gap.remove();
      current.cards.forEach(card => layoutAnimations.get(card)?.cancel());
      // Undo the temporary layout before applying the deferred Rust render.
      current.all.forEach(card => current.grid.append(card));
      current.moving.forEach(card => { card.hidden = false; card.classList.remove('reorder-landing'); });
      current.grid.inert = wasInert;
      settling = false;
      finish();
      if (current.card.isConnected) current.card.querySelector('.game-face')?.focus({ preventScroll: true });
    }
  }

  function end(save) {
    if (hold) { clearTimeout(hold.timer); hold = null; }
    if (!drag) return;
    const current = drag;
    drag = null;
    cancelAnimationFrame(frame);
    if (current.grid.hasPointerCapture(current.pointerId)) current.grid.releasePointerCapture(current.pointerId);
    if (save && current.valid) {
      void land(current);
      return;
    }
    current.preview.remove();
    transitionLayout(current.cards, () => {
      current.gap.remove();
      current.moving.forEach(card => { card.hidden = false; });
    });
    finish();
  }

  document.addEventListener('pointerdown', event => {
    if (settling) return;
    const card = event.target.closest('.game-card');
    const grid = card?.closest('.game-grid[data-custom-sort="true"]');
    if (!grid || event.button !== 0 || event.isPrimary === false || event.altKey || event.target.closest('.card-action,.card-remove') || document.querySelector('[aria-modal="true"],.library-bar [data-action=batch-toggle]:disabled')) return;
    end(false);
    hold = { card, grid, pointerId: event.pointerId, x: event.clientX, y: event.clientY, timer: setTimeout(begin, 380) };
  });
  document.addEventListener('pointermove', event => {
    if (hold && Math.hypot(event.clientX - hold.x, event.clientY - hold.y) > 6) { clearTimeout(hold.timer); hold = null; }
    if (drag && drag.pointerId === event.pointerId) { drag.x = event.clientX; drag.y = event.clientY; event.preventDefault(); }
  });
  document.addEventListener('pointerup', event => {
    if (drag?.pointerId === event.pointerId) {
      // A release can arrive before the last move reaches the animation frame.
      cancelAnimationFrame(frame);
      drag.x = event.clientX;
      drag.y = event.clientY;
      move();
      end(true);
    } else if (hold?.pointerId === event.pointerId) end(false);
  });
  document.addEventListener('pointercancel', event => {
    if (drag?.pointerId === event.pointerId || hold?.pointerId === event.pointerId) end(false);
  });
  document.addEventListener('lostpointercapture', event => { if (drag?.pointerId === event.pointerId) end(false); });
  window.addEventListener('blur', () => end(false));
  window.addEventListener('resize', () => end(false));
  document.addEventListener('keydown', event => {
    if (settling) return;
    if (event.key === 'Escape' && (hold || drag)) { event.preventDefault(); event.stopImmediatePropagation(); end(false); return; }
    const card = event.target.closest('.game-card');
    const grid = card?.closest('.game-grid[data-custom-sort="true"]');
    if (!grid || !event.altKey || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    event.preventDefault();
    const ids = JSON.parse(idsFor(card.dataset.gameId) || '[]');
    const cards = [...grid.querySelectorAll('.game-card')];
    const index = cards.indexOf(card);
    const candidates = cards.filter(item => !ids.includes(item.dataset.gameId));
    const before = event.key === 'ArrowLeft' ? cards.slice(0, index).reverse().find(item => candidates.includes(item)) : cards.slice(index + 1).filter(item => candidates.includes(item))[1];
    if (event.key === 'ArrowLeft' && !before) return;
    commit(ids, before?.dataset.gameId || null);
  }, true);
}
