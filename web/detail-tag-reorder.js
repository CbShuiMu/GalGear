import {LONG_PRESS_DELAY_MS, movedPastLongPressSlop} from './press-gesture.js';

// 点击与长按都命中同一个标签主体（包含留白），两侧按钮仍使用各自操作。
export function detailTagClickTarget(event) {
  const chip = event.target.closest?.('.hero-tags .detail-tag-chip:not(.tag-input-chip)');
  if (!chip) return null;
  return event.target.closest('[data-action]') || chip.querySelector('.tag-button');
}

// 只管理手势、占位和动画；标签顺序与保存结果由 Rust 决定。
export function installDetailTagReorder({commit, render}) {
  const easing = 'cubic-bezier(.4,0,.2,1)';
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const animations = new WeakMap();
  let hold = null, drag = null, settling = false, frame = 0;
  let deferred = null, suppressUntil = 0;
  let cancelledPointer = null;

  function cancelHold(suppressClick = false) {
    if (hold && suppressClick) cancelledPointer = hold.pointerId;
    if (hold) clearTimeout(hold.timer);
    hold = null;
  }

  function layout(nodes, change) {
    const old = nodes.map(node => node.getBoundingClientRect());
    nodes.forEach(node => animations.get(node)?.cancel());
    change();
    const next = nodes.map(node => node.getBoundingClientRect());
    if (!reduced()) nodes.forEach((node, index) => {
      const dx = old[index].x - next[index].x, dy = old[index].y - next[index].y;
      if (!dx && !dy) return;
      animations.set(node, node.animate([
        {transform: `translate(${dx}px,${dy}px)`}, {transform: 'none'}
      ], {duration: 260, easing}));
    });
    return next;
  }

  function arrange(current, before) {
    const boxes = layout(current.others, () => current.list.insertBefore(current.chip, before));
    const bounds = current.list.getBoundingClientRect();
    // 命中检测使用未变换的布局坐标，避免让位动画反复改变落点。
    current.slots = boxes.map(box => ({
      left: box.left - bounds.left, top: box.top - bounds.top,
      right: box.right - bounds.left, bottom: box.bottom - bounds.top
    }));
    current.bounds = bounds;
    current.gap = current.chip.getBoundingClientRect();
    current.slotScroll = current.pane.scrollTop;
    current.before = before;
  }

  function update(current, now = performance.now()) {
    const {x, y, anchor, pane, viewport, bounds, gap} = current;
    const elapsed = Math.min(32, now - current.lastFrame);
    current.lastFrame = now;
    const previousScroll = pane.scrollTop;
    if (y > viewport.bottom - 32) pane.scrollTop += elapsed * .48;
    else if (y < viewport.top + 32) pane.scrollTop -= elapsed * .48;
    const scroll = pane.scrollTop;
    current.scrolling = scroll !== previousScroll;
    // 连续跟随只更新合成层的位置；拾起动画在子节点上单独播放，不重启缓动。
    if (current.paintedX !== x || current.paintedY !== y) {
      anchor.style.transform = `translate3d(${x - current.width / 2}px,${y - current.height / 2 - 8}px,0)`;
      current.paintedX = x; current.paintedY = y;
    }
    const offset = scroll - current.slotScroll;
    current.valid = x >= bounds.left - 12 && x <= bounds.right + 12
      && y >= Math.max(bounds.top - offset - 16, viewport.top) && y <= Math.min(bounds.bottom - offset + 20, viewport.bottom);
    if (!current.valid) return;
    // 换行后标签会出现在指针下面；静止指针不能再次触发让位，导致落点自行漂移。
    if (current.hit?.x === x && current.hit.y === y && current.hit.scroll === scroll) return;
    current.hit = {x, y, scroll};
    if (x >= gap.left && x <= gap.right && y >= gap.top - offset && y <= gap.bottom - offset) return;
    const localX = x - bounds.left, localY = y - bounds.top + offset;
    const index = current.slots.findIndex(box => localY < box.top
      || localY <= box.bottom && localX < (box.left + box.right) / 2);
    const before = index < 0 ? current.tail : current.others[index];
    if (before !== current.before) arrange(current, before);
  }

  function schedule() {
    if (!frame) frame = requestAnimationFrame(tick);
  }

  function tick(now) {
    frame = 0;
    if (!drag) return;
    update(drag, now);
    if (drag.scrolling) schedule();
  }

  function begin() {
    const current = hold;
    hold = null;
    if (!current?.chip.isConnected || !current.label.isConnected || current.list.dataset.tagReorder !== 'true') return;
    const box = current.label.getBoundingClientRect();
    const layer = document.createElement('div');
    layer.className = 'reference-detail tag-drag-layer';
    layer.setAttribute('aria-hidden', 'true');
    layer.inert = true;
    const anchor = document.createElement('div');
    anchor.className = 'tag-drag-anchor';
    const left = current.x - box.width / 2, top = current.y - box.height / 2 - 8;
    anchor.style.transform = `translate3d(${left}px,${top}px,0)`;
    const preview = current.label.cloneNode(true);
    preview.removeAttribute('id');
    preview.removeAttribute('data-action');
    preview.removeAttribute('data-value');
    preview.classList.add('tag-drag-preview');
    Object.assign(preview.style, {
      width: `${box.width}px`, height: `${box.height}px`
    });
    anchor.append(preview);
    layer.append(anchor);
    current.list.closest('.app-shell').append(layer);
    const pickup = reduced() ? null : preview.animate([
      {transform: `translate(${box.x - left}px,${box.y - top}px) scale(1)`},
      {transform: 'scale(1.06)'}
    ], {duration: 160, easing});
    current.list.classList.add('tag-reordering');
    current.chip.classList.add('tag-reorder-slot');
    document.getSelection()?.removeAllRanges();
    const chips = [...current.list.querySelectorAll('.detail-tag-chip')];
    drag = {...current, layer, anchor, preview, pickup, width: box.width, height: box.height,
      viewport: current.pane.getBoundingClientRect(), lastFrame: performance.now() - 16,
      others: chips.filter(chip => chip !== current.chip),
      originalNext: current.chip.nextElementSibling,
      tail: current.list.querySelector('.tag-inline-actions'), valid: true};
    arrange(drag, drag.originalNext);
    current.list.setPointerCapture(current.pointerId);
    tick();
  }

  async function land(current, save) {
    settling = true;
    current.list.inert = true;
    try {
      if (!save) arrange(current, current.originalNext);
      const origin = current.preview.getBoundingClientRect();
      const target = current.label.getBoundingClientRect();
      current.pickup?.cancel();
      current.layer.append(current.preview);
      Object.assign(current.preview.style, {
        transition: 'none', left: `${origin.x}px`, top: `${origin.y}px`,
        width: `${origin.width}px`, height: `${origin.height}px`, transform: 'none'
      });
      if (!reduced()) await current.preview.animate([
        {transform: 'none'},
        {transform: `translate(${target.x - origin.x}px,${target.y - origin.y}px) scale(${target.width / origin.width},${target.height / origin.height})`}
      ], {duration: 300, easing, fill: 'forwards'}).finished.catch(() => {});
      current.chip.classList.remove('tag-reorder-slot');
      current.layer.remove();
      if (save && current.before !== current.originalNext) {
        await commit({id: current.id, tag: current.label.dataset.value,
          before: current.before?.querySelector('.tag-button')?.dataset.value ?? null});
      }
    } finally {
      current.layer.remove();
      current.chip.classList.remove('tag-reorder-slot');
      current.list.classList.remove('tag-reordering');
      current.list.inert = false;
      settling = false;
      suppressUntil = performance.now() + 400;
      const html = deferred;
      deferred = null;
      // 保存失败或操作被 Rust 拒绝时回到权威顺序，也保留让位动画。
      layout([...current.others, current.chip], () => {
        current.list.insertBefore(current.chip, current.originalNext);
        if (html !== null) render(html);
      });
      // 鼠标拖放不恢复标签焦点，否则 :focus-within 会在移开鼠标后仍展开两侧操作。
      // 键盘排序走独立入口，继续保留其焦点。
      if (current.chip.contains(document.activeElement)) document.activeElement.blur();
    }
  }

  function end(save) {
    cancelHold(true);
    if (!drag) return;
    const current = drag;
    drag = null;
    cancelAnimationFrame(frame);
    frame = 0;
    if (current.list.hasPointerCapture(current.pointerId)) current.list.releasePointerCapture(current.pointerId);
    void land(current, save && current.valid);
  }

  function handle(target) {
    const label = detailTagClickTarget({target});
    if (label?.dataset.action !== 'detail-tag') return null;
    const list = label?.closest('.hero-tags .game-tags[data-tag-reorder="true"]');
    if (!list || document.querySelector('[aria-modal="true"]')) return null;
    return {label, list, chip: label.parentElement,
      pane: list.closest('.detail-scroll'), id: list.closest('[data-detail-game]').dataset.detailGame};
  }

  document.addEventListener('pointerdown', event => {
    if (settling || drag || event.button !== 0 || event.isPrimary === false || event.altKey) return;
    cancelHold();
    cancelledPointer = null;
    const current = handle(event.target);
    if (!current) return;
    hold = {...current, pointerId: event.pointerId, x: event.clientX, y: event.clientY,
      timer: setTimeout(begin, LONG_PRESS_DELAY_MS)};
  });
  document.addEventListener('pointermove', event => {
    if (hold?.pointerId === event.pointerId && movedPastLongPressSlop(hold, event)) cancelHold(true);
    if (drag?.pointerId === event.pointerId) {
      drag.x = event.clientX; drag.y = event.clientY;
      schedule();
      event.preventDefault();
    }
  }, {passive: false});
  document.addEventListener('pointerup', event => {
    if (drag?.pointerId === event.pointerId) {
      drag.x = event.clientX; drag.y = event.clientY;
      update(drag);
      end(true);
    } else if (hold?.pointerId === event.pointerId) cancelHold();
    else if (cancelledPointer === event.pointerId) {
      cancelledPointer = null;
      suppressUntil = performance.now() + 400;
    }
  });
  for (const name of ['pointercancel', 'lostpointercapture']) document.addEventListener(name, event => {
    if (drag?.pointerId === event.pointerId || hold?.pointerId === event.pointerId) end(false);
  });
  document.addEventListener('scroll', event => {
    if (hold && (hold.chip.contains(event.target) || event.target.contains?.(hold.chip))) cancelHold(true);
    if (drag && event.target === drag.pane) schedule();
  }, true);
  document.addEventListener('wheel', event => { if (hold?.chip.contains(event.target)) cancelHold(true); }, {passive: true});
  window.addEventListener('blur', () => end(false));
  window.addEventListener('resize', () => end(false));
  document.addEventListener('click', event => {
    if (drag || settling || performance.now() < suppressUntil) {
      event.preventDefault(); event.stopImmediatePropagation();
    }
  }, true);
  document.addEventListener('dragstart', event => {
    if (hold || drag || handle(event.target)) event.preventDefault();
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && (hold || drag || settling)) {
      event.preventDefault(); event.stopImmediatePropagation(); end(false); return;
    }
    if (drag || settling || !event.altKey || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    const current = handle(event.target);
    if (!current) return;
    event.preventDefault(); event.stopImmediatePropagation();
    const chips = [...current.list.querySelectorAll('.detail-tag-chip')];
    const index = chips.indexOf(current.chip), left = event.key === 'ArrowLeft';
    if (left && index === 0 || !left && index === chips.length - 1) return;
    void commit({id: current.id, tag: current.label.dataset.value,
      before: chips[index + (left ? -1 : 2)]?.querySelector('.tag-button')?.dataset.value ?? null});
  }, true);

  return {
    deferRender(html) {
      if (drag || settling) { deferred = html; return true; }
      // 与卡片一致：后台重绘不重置长按计时；begin 再校验节点与可排序状态。
      return false;
    }
  };
}
