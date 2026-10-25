// Only the DOM / WebAssembly / Tauri transport bridge lives in JavaScript.
// Rendering, app state, event decisions and all application logic live in Rust.
import init, * as ui from './pkg/galgear_ui.js';

let wasmReady = false;
let previousDialog = false;
let dialogReturnFocus = null;
let toastTimer = null;
let downloadNoticeTimer = null;
let moreObserver = null;
let previousChoice = null;
let choiceScroll = 0;
let choiceReturnFocus = null;
let restoringScroll = false;
let libraryReturnFocus = null;
let previousLibraryDialog = false;
let previousSetup = null;
let setupReturnFocus = null;
let previousEditor = false;
let previousContext = false;
let contextReturnId = null;
let editorReturnFocus = null;
let dragPayload = '';
let deferredDragFrame = null;
let pointerDrag = null;
let marqueeDrag = null;
let marqueeFrame = null;
let dragPreview = null;
let suppressCardClickUntil = 0;
let dragHoverTimer = null;
let dragHoverKey = null;
let minimumRequest = '';
let observedLibraryGrid = null;
const libraryGridObserver = new ResizeObserver(() => alignLibraryToolbar());
const pagePositions = new Map();
let resetPageScroll = false;

// Keep live inputs, posters and scroll containers across state updates.
// Replacing the root restarted width transitions and made detail images reload.
function renderKey(node) {
  if (node.nodeType !== Node.ELEMENT_NODE) return null;
  return node.id || node.dataset.renderKey ||
    (node.dataset.gameId ? `game:${node.dataset.gameId}` : null) ||
    (node.dataset.action ? `action:${node.dataset.action}:${node.dataset.value || ''}` : null);
}
function compatible(current, next) {
  return current.nodeType === next.nodeType && current.nodeName === next.nodeName &&
    renderKey(current) === renderKey(next);
}
function reconcile(current, next) {
  if (current.nodeType !== Node.ELEMENT_NODE) {
    if (current.nodeValue !== next.nodeValue) current.nodeValue = next.nodeValue;
    return;
  }
  for (const attribute of [...current.attributes]) {
    if (!next.hasAttribute(attribute.name) && attribute.name !== 'style' &&
        !(current instanceof HTMLDetailsElement && attribute.name === 'open')) {
      current.removeAttribute(attribute.name);
    }
  }
  for (const attribute of next.attributes) {
    if (current.getAttribute(attribute.name) !== attribute.value) current.setAttribute(attribute.name, attribute.value);
  }
  if (current instanceof HTMLInputElement) {
    if (current !== document.activeElement && current.value !== next.value) current.value = next.value;
    current.checked = next.checked;
  }
  if (current instanceof HTMLTextAreaElement) {
    if (current !== document.activeElement && current.value !== next.value) current.value = next.value;
    return;
  }
  reconcileChildren(current, next);
  if (current instanceof HTMLSelectElement && current.value !== next.value) current.value = next.value;
}
function reconcileChildren(current, next) {
  let cursor = current.firstChild;
  for (const desired of [...next.childNodes]) {
    let match = cursor;
    if (!match || !compatible(match, desired)) {
      const key = renderKey(desired);
      match = key ? [...current.childNodes].find(node => compatible(node, desired)) : null;
    }
    if (match) {
      if (match !== cursor) current.insertBefore(match, cursor);
      reconcile(match, desired);
      cursor = match.nextSibling;
    } else {
      current.insertBefore(desired.cloneNode(true), cursor);
    }
  }
  while (cursor) {
    const obsolete = cursor;
    cursor = cursor.nextSibling;
    obsolete.remove();
  }
}

function alignLibraryToolbar() {
  const page = document.querySelector('.library-page');
  const bar = page?.querySelector('.library-bar');
  const grid = page?.querySelector('.game-grid');
  if (grid !== observedLibraryGrid) {
    libraryGridObserver.disconnect();
    observedLibraryGrid = grid;
    if (grid) libraryGridObserver.observe(grid);
  }
  if (!bar) return;
  const cards = grid?.querySelectorAll('.game-card');
  if (!cards?.length) {
    bar.style.removeProperty('width');
    return;
  }
  const right = Math.max(...[...cards].map(card => card.getBoundingClientRect().right));
  const bounds = grid.getBoundingClientRect();
  const cardWidth = cards[0].getBoundingClientRect().width;
  const gap = parseFloat(getComputedStyle(grid).columnGap) || 0;
  const minimumRight = bounds.left + Math.min(bounds.width, cardWidth * 4 + gap * 3);
  const width = Math.max(0, Math.max(right, minimumRight) - bar.getBoundingClientRect().left);
  const value = `${width}px`;
  if (bar.style.width !== value) bar.style.width = value;
}

function measureLibraryBounds() {
  alignLibraryToolbar();
  const page = document.querySelector('.library-page');
  const bar = page?.querySelector('.library-bar');
  const shell = document.querySelector('.app-shell');
  const pane = document.getElementById('page-scroll');
  if (!bar || !shell || !pane) return;
  const style = getComputedStyle(page);
  const cardMinimum = parseFloat(style.getPropertyValue('--library-card-min-width'));
  const gap = parseFloat(style.getPropertyValue('--library-column-gap'));
  const inset = parseFloat(style.getPropertyValue('--library-min-content-inset'));
  const sidebar = pane.getBoundingClientRect().left - shell.getBoundingClientRect().left;
  const gutter = pane.offsetWidth - pane.clientWidth;
  // Use the four-column baseline so enlarging the window never raises its minimum.
  const width = Math.ceil(cardMinimum * 4 + gap * 3 + inset * 2 + sidebar + gutter);
  const bounds = { width, height: 640 };
  const key = JSON.stringify(bounds);
  // Keep bounds available to isolated UI verification without mocking window internals.
  shell.dataset.minimumWidth = String(bounds.width);
  if (window.__TAURI__?.core && key !== minimumRequest) {
    minimumRequest = key;
    window.__TAURI__.core.invoke('set_library_bounds', bounds).catch(error => {
      minimumRequest = '';
      console.error('Unable to apply library window bounds', error);
    });
  }
}

function placeChoice() {
  const menu = document.querySelector('.choice-popup');
  const anchor = menu && document.getElementById(menu.dataset.choiceAnchor);
  if (!anchor) return;
  const box = anchor.getBoundingClientRect();
  const optionWidth=Math.max(0,...[...menu.querySelectorAll('.choice-copy>span')].map(node=>node.scrollWidth+64));
  const width = Math.min(Math.max(box.width, menu.classList.contains('game-menu') ? 340 : menu.classList.contains('setup-menu') ? Math.min(560, optionWidth) : optionWidth), innerWidth - 16);
  menu.style.width = `${width}px`;
  menu.style.maxHeight = `${Math.min(340, innerHeight - 24)}px`;
  const height = menu.getBoundingClientRect().height;
  const below = innerHeight - box.bottom - 12;
  const above = box.top - 12;
  const bottomSide = below >= height || below >= above;
  menu.style.maxHeight = `${Math.max(70, Math.min(height, bottomSide ? below : above))}px`;
  menu.style.left = `${Math.max(8, Math.min(box.left, innerWidth - width - 8))}px`;
  menu.style.top = `${bottomSide ? box.bottom + 6 : Math.max(8, box.top - menu.getBoundingClientRect().height - 6)}px`;
}
let detailReturnAnimation = null;
function closeDetail() {
  if (detailReturnAnimation) return;
  const overlay = document.querySelector('.detail-overlay');
  if (!overlay || matchMedia('(prefers-reduced-motion: reduce)').matches || !overlay.animate) {
    ui.dispatch('close-detail', '');
    return;
  }
  overlay.inert = true;
  const animation = overlay.animate(
    [{ transform: 'translateX(0)' }, { transform: 'translateX(100%)' }],
    { duration: 240, easing: 'cubic-bezier(.4,0,.2,1)', fill: 'forwards' }
  );
  detailReturnAnimation = animation;
  animation.finished.catch(() => {}).then(() => {
    if (detailReturnAnimation !== animation) return;
    detailReturnAnimation = null;
    overlay.inert = false;
    if (document.querySelector('.detail-overlay') === overlay) ui.dispatch('close-detail', '');
  });
}

let activeThemeTransition = null;
let segmentedAppearanceChange = false;
const initializedSegmentThumbs = new WeakSet();

function syncSegmentedControls(animate = true) {
  for (const group of document.querySelectorAll('.segmented-control')) {
    const selected = group.querySelector('.segment[aria-pressed="true"]');
    const thumb = group.querySelector('.segmented-thumb');
    if (!selected || !thumb) continue;
    const groupBox = group.getBoundingClientRect();
    const selectedBox = selected.getBoundingClientRect();
    const immediate = !animate || !initializedSegmentThumbs.has(thumb);
    if (immediate) thumb.style.transition = 'none';
    thumb.style.width = `${selectedBox.width}px`;
    thumb.style.transform = `translateX(${selectedBox.left - groupBox.left - group.clientLeft}px)`;
    if (immediate) {
      thumb.getBoundingClientRect();
      thumb.style.transition = '';
    }
    initializedSegmentThumbs.add(thumb);
  }
}

function renderFrame(html) {
    restoringScroll = true;
    moreObserver?.disconnect();
    const active = document.activeElement;
    const focusId = active?.id;
    const selection = active?.selectionStart;
    const selectionEnd = active?.selectionEnd;
    const selectionDirection = active?.selectionDirection;
    const focusInChoice = !!active?.closest('.choice-popup') || active?.id === previousChoice;
    choiceScroll = document.querySelector('.choice-list')?.scrollTop || 0;
    const currentPage = document.getElementById('page-scroll');
    if (currentPage) pagePositions.set(currentPage.dataset.scrollKey, {
      top: currentPage.scrollTop, left: currentPage.scrollLeft,
      details: [...currentPage.querySelectorAll('details[open]')].map(node => renderKey(node) || node.className)
    });
    const setupTable = document.getElementById('setup-table-scroll');
    const setupScroll = setupTable ? { top: setupTable.scrollTop, left: setupTable.scrollLeft } : null;
    const treeScroll = document.getElementById('collection-tree')?.scrollTop || 0;
    const treeLeft = document.getElementById('collection-tree')?.scrollLeft || 0;
    const hasEditor = html.includes('class="editor-overlay"');
    if (hasEditor && !previousEditor) editorReturnFocus = active ? {id:active.id,action:active.dataset.action,value:active.dataset.value} : null;
    const editorScroll = document.querySelector('.editor-body')?.scrollTop || 0;
    const hasLibraryDialog = html.includes('class="library-overlay"');
    if (hasLibraryDialog && !previousLibraryDialog) {
      libraryReturnFocus = active ? { id: focusId || active.closest('[data-game-id]')?.querySelector('.game-face')?.id, action: active.dataset.action, value: active.dataset.value } : null;
    }
    const detailsScroll = document.querySelector('.detail-scroll')?.scrollTop || 0;
    const openDetails = [...document.querySelectorAll('details[open]')].filter(node => !currentPage?.contains(node)).map(node => node.className);
    const hasDialog = html.includes('class="detail-overlay');
    if (hasDialog && !previousDialog) dialogReturnFocus = active?.dataset?.value;
    const template = document.createElement('template');
    template.innerHTML = html;
    reconcileChildren(document.getElementById('app'), template.content);
    const shell = document.querySelector('.app-shell');
    if (shell) {
      shell.style.setProperty('--cover-scale', shell.dataset.coverScale);
      shell.style.setProperty('--ui-font', `${shell.dataset.fontSize}px`);
    }
    document.querySelectorAll('[data-game-id]').forEach(card => { card.draggable = false; });
    for (const node of document.querySelectorAll('details')) if (!node.closest('#page-scroll') && openDetails.includes(node.className)) node.open = true;
    const language = document.querySelector('.app-shell')?.getAttribute('lang');
    if (language) document.documentElement.lang = language;
    const pane = document.getElementById('page-scroll');
    const pagePosition = pane && pagePositions.get(pane.dataset.scrollKey);
    if (pane && pagePosition) {
      for (const node of pane.querySelectorAll('details')) node.open = pagePosition.details.includes(renderKey(node) || node.className);
    }
    const pageTop = resetPageScroll ? 0 : pagePosition?.top || 0;
    const pageLeft = resetPageScroll ? 0 : pagePosition?.left || 0;
    resetPageScroll = false;
    if (pane) { pane.scrollTop = pageTop; pane.scrollLeft = pageLeft; }
    const restoreSetupScroll = () => {
      const table = document.getElementById('setup-table-scroll');
      if (table && setupScroll) { table.scrollTop = setupScroll.top; table.scrollLeft = setupScroll.left; }
    };
    restoreSetupScroll();
    const tree = document.getElementById('collection-tree');
    if (tree) { tree.scrollTop = treeScroll; tree.scrollLeft = treeLeft; }
    const details = document.querySelector('.detail-scroll');
    if (details) details.scrollTop = detailsScroll;
    if (hasDialog && !previousDialog) document.getElementById('close-detail')?.focus();
    else if (!hasDialog && previousDialog) {
      document.getElementById(`game-${dialogReturnFocus}`)?.focus({ preventScroll: true });
    } else if (focusId) {
      const next = document.getElementById(focusId);
      // A retained input already owns its caret and IME composition session.
      if (next && next !== active) {
        next.focus({ preventScroll: true });
        if (typeof selection === 'number' && next.setSelectionRange) {
          next.setSelectionRange(selection, selectionEnd ?? selection, selectionDirection ?? 'none');
        }
      }
    }
    previousDialog = hasDialog;
    const choice = document.querySelector('.choice-popup');
    const choiceId = choice?.dataset.choiceAnchor || null;
    if (choice) {
      placeChoice();
      const list = document.getElementById('choice-list');
      if (list) list.scrollTop = choiceScroll;
      if (choiceId !== previousChoice) (document.getElementById('choice-query') || list)?.focus({ preventScroll: true });
      document.querySelector('.choice-option.active')?.scrollIntoView({ block: 'nearest' });
    } else if (previousChoice && focusInChoice) choiceReturnFocus = previousChoice;
    if (!choice && choiceReturnFocus) {
      if (focusId && !['choice-list', 'choice-query', choiceReturnFocus].includes(focusId)) choiceReturnFocus = null;
      const trigger = choiceReturnFocus && document.getElementById(choiceReturnFocus);
      if (trigger && !trigger.disabled) { trigger.focus({ preventScroll: true }); choiceReturnFocus = null; }
      else if (!trigger) choiceReturnFocus = null;
    }
    previousChoice = choiceId;
    if (hasLibraryDialog && !previousLibraryDialog) (document.getElementById('folder-name') || document.getElementById('folder-query') || document.getElementById('tag-query') || document.getElementById('game-query') || document.getElementById('library-dialog-close'))?.focus();
    else if (!hasLibraryDialog && previousLibraryDialog) {
      const origin=libraryReturnFocus;
      const target=(origin?.id && document.getElementById(origin.id)) || [...document.querySelectorAll('[data-action]')].find(node=>node.dataset.action===origin?.action && node.dataset.value===origin?.value) || document.querySelector('.collection-bar [data-action=folder-new]');
      target?.focus({ preventScroll: true });
    }
    previousLibraryDialog = hasLibraryDialog;
    document.querySelectorAll('.sidebar,.page-scroll,.detail-overlay').forEach(node => {
      node.inert = hasEditor || (node.matches('.detail-overlay') && !!detailReturnAnimation);
    });
    const editorBody = document.querySelector('.editor-body');
    if (editorBody) editorBody.scrollTop = editorScroll;
    if (hasEditor && !previousEditor) document.querySelector('.editor-body input,.editor-body textarea,.editor-dialog button')?.focus();
    else if (!hasEditor && previousEditor && editorReturnFocus) (document.getElementById(editorReturnFocus.id) || [...document.querySelectorAll('[data-action]')].find(node=>node.dataset.action===editorReturnFocus.action && node.dataset.value===editorReturnFocus.value))?.focus({preventScroll:true});
    previousEditor = hasEditor;
    const setup = document.querySelector('.setup-dialog');
    const setupKey = setup?.dataset.renderKey || null;
    if (setupKey && !previousSetup) setupReturnFocus = active;
    document.querySelectorAll('.sidebar,.page-scroll,.detail-overlay,.editor-overlay,.library-overlay').forEach(node => {
      node.inert = !!setup || (node.matches('.sidebar,.page-scroll,.detail-overlay') && (hasEditor || (node.matches('.detail-overlay') && !!detailReturnAnimation)));
    });
    if (setupKey && setupKey !== previousSetup) setup.querySelector('.choice-trigger')?.focus({ preventScroll: true });
    else if (!setupKey && previousSetup && setupReturnFocus?.isConnected) setupReturnFocus.focus({ preventScroll: true });
    previousSetup = setupKey;
    const context = document.querySelector('.game-context-menu');
    if (context) {
      const box=context.getBoundingClientRect();
      context.style.left=`${Math.max(8,Math.min(Number(context.dataset.contextX),innerWidth-box.width-8))}px`;
      context.style.top=`${Math.max(8,Math.min(Number(context.dataset.contextY),innerHeight-box.height-8))}px`;
      context.classList.toggle('flip-submenu',context.getBoundingClientRect().right+180>innerWidth);
      const sub=context.querySelector('.context-submenu');
      sub.style.visibility='hidden';sub.style.display='block';
      const state=context.querySelector('.context-state').getBoundingClientRect();
      sub.style.top=`${Math.min(0,innerHeight-state.top-sub.getBoundingClientRect().height-8)}px`;
      sub.style.visibility='';sub.style.display='';
      if(!previousContext) {contextReturnId=active?.closest('[data-game-id]')?.dataset.gameId;context.querySelector('button')?.focus();}
      previousContext=true;
    } else if(previousContext) {
      if(contextReturnId)document.getElementById(`game-${contextReturnId}`)?.focus({preventScroll:true});
      previousContext=false;contextReturnId=null;
    }
    measureLibraryBounds();
    syncSegmentedControls();
    if (pane) { pane.scrollTop = pageTop; pane.scrollLeft = pageLeft; }
    restoreSetupScroll();
    requestAnimationFrame(() => { restoringScroll = false; });
    const more = document.querySelector('[data-action="load-more"]');
    if (more && !hasDialog) {
      moreObserver = new IntersectionObserver(entries => {
        if (entries.some(e => e.isIntersecting) && wasmReady) {
          moreObserver.disconnect();
          ui.dispatch('load-more', '');
        }
      }, { root: document.getElementById('page-scroll'), rootMargin: '300px' });
      moreObserver.observe(more);
    }
  
}

window.GalGearBridge = {
  local_date(seconds) {
    const date = new Date(seconds * 1000);
    if (!Number.isFinite(date.getTime())) return '';
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  },
  schedule_toast_expiry(token, milliseconds) {
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => ui.expire_toast(token), milliseconds);
  },
  schedule_download_notice_expiry(token, milliseconds) {
    clearTimeout(downloadNoticeTimer);
    downloadNoticeTimer = setTimeout(() => ui.expire_download_notice(token), milliseconds);
  },
  reset_scroll() {
    // Apply explicit resets to the incoming view after its content is reconciled.
    resetPageScroll = true;
  },
  render(html) {
    if (marqueeDrag) { deferredDragFrame = html; return; }
    if (dragPayload) {
      // Expanding a drop target must not replace the card that owns the native drag.
      deferredDragFrame = html;
      const parsed = new DOMParser().parseFromString(html, 'text/html');
      const next = parsed.querySelector('.collections-nav');
      const current = document.querySelector('.collections-nav');
      if (next && current) {
        const oldTree = current.querySelector('.collection-tree');
        const top = oldTree?.scrollTop || 0, left = oldTree?.scrollLeft || 0;
        current.replaceWith(next);
        const newTree = next.querySelector('.collection-tree');
        if (newTree) { newTree.scrollTop = top; newTree.scrollLeft = left; }
      }
      return;
    }
    const theme = /data-theme="(light|dark)"/.exec(html)?.[1];
    const colorTheme = /data-color-theme="(pink|white-pink)"/.exec(html)?.[1];
    const previous = document.documentElement.dataset.theme;
    const previousColorTheme = document.documentElement.dataset.colorTheme;
    const apply = () => {
      if (theme) document.documentElement.dataset.theme = theme;
      if (colorTheme) document.documentElement.dataset.colorTheme = colorTheme;
      renderFrame(html);
    };
    if (!previous || !theme || (previous === theme && previousColorTheme === colorTheme) || matchMedia('(prefers-reduced-motion: reduce)').matches) { apply(); return; }
    // Keep the live selection background visible while a settings segment moves.
    if (segmentedAppearanceChange) {
      segmentedAppearanceChange = false;
      activeThemeTransition?.skipTransition();
      document.querySelector('.theme-fallback')?.remove();
      apply();
      return;
    }
    activeThemeTransition?.skipTransition();
    if (typeof document.startViewTransition === 'function') {
      const transition = document.startViewTransition(apply);
      activeThemeTransition = transition;
      transition.ready.catch(() => {});
      transition.finished.catch(() => {}).finally(() => { if (activeThemeTransition === transition) activeThemeTransition = null; });
    } else {
      document.querySelector('.theme-fallback')?.remove();
      const old = document.querySelector('.app-shell')?.cloneNode(true);
      const positions = ['.page-scroll', '.detail-scroll'].map(selector => [selector, document.querySelector(selector)?.scrollTop || 0]);
      if (old) {
        old.classList.add('theme-fallback'); old.setAttribute('aria-hidden', 'true'); old.inert = true;
      }
      apply();
      if (old) { document.body.append(old); for (const [selector, top] of positions) { const pane = old.querySelector(selector); if (pane) pane.scrollTop = top; } old.addEventListener('animationend', () => old.remove(), { once: true }); setTimeout(() => old.remove(), 600); }
    }
  },
  async request(id, command, payload) {
    try {
      if (!window.__TAURI__?.core) throw { key: 'error.desktop', args: {} };
      const result = await window.__TAURI__.core.invoke(command, JSON.parse(payload));
      ui.resolve(id, true, JSON.stringify(result ?? null));
    } catch (error) {
      // Bootstrap may call synchronously during wasm initialization.
      const payload = error && typeof error === 'object' && 'key' in error
        ? JSON.stringify(error) : String(error);
      queueMicrotask(() => ui.resolve(id, false, payload));
    }
  }
};

document.addEventListener('click', event => {
  if (!wasmReady) return;
  if (performance.now() < suppressCardClickUntil) { event.preventDefault(); event.stopPropagation(); return; }
  if (document.querySelector('.game-context-menu') && !event.target.closest('.game-context-menu')) ui.dispatch('context-close','');
  const element = event.target.closest('[data-action]');
  if (['toggle-theme', 'set-theme', 'set-color-theme'].includes(element?.dataset.action)) {
    segmentedAppearanceChange = !!element.closest('.segmented-control');
    document.documentElement.style.setProperty('--theme-x', `${event.clientX || element.getBoundingClientRect().x + element.clientWidth / 2}px`);
    document.documentElement.style.setProperty('--theme-y', `${event.clientY || element.getBoundingClientRect().y + element.clientHeight / 2}px`);
  }
  if (document.querySelector('.choice-popup') && !event.target.closest('.choice-popup,.choice-trigger')) ui.dispatch('choice-close', '');
  if (!element || element.disabled) return;
  event.preventDefault();
  const addMenu = element.closest('.add-menu');
  if (addMenu) addMenu.open = false;
  if (element.dataset.action === 'close-detail') { closeDetail(); return; }
  if (element.dataset.action === 'detail-top') {
    document.querySelector('.detail-scroll')?.scrollTo({ top: 0, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
    return;
  }
  ui.dispatch(element.dataset.action, element.dataset.value || '');
  if (['tag-filter', 'tag-filter-remove'].includes(element.dataset.action)) {
    document.getElementById('tag-query')?.focus({ preventScroll: true });
  }
});
document.addEventListener('pointerout', event => {
  const menu = event.target.closest?.('.add-menu[open]');
  if (menu && !menu.contains(event.relatedTarget)) menu.open = false;
});
document.addEventListener('focusout', event => {
  const menu = event.target.closest?.('.add-menu[open]');
  if (menu && !menu.contains(event.relatedTarget)) menu.open = false;
});
document.addEventListener('input', event => {
  if (wasmReady && !event.isComposing && event.target.dataset.input) ui.input_changed(event.target.dataset.input, event.target.value);
});
document.addEventListener('compositionend', event => {
  if (wasmReady && event.target.dataset.input) ui.input_changed(event.target.dataset.input, event.target.value);
});
document.addEventListener('change', event => {
  if (wasmReady && event.target.dataset.select) ui.dispatch(event.target.dataset.select, event.target.value);
});
document.addEventListener('keydown', event => {
  if (wasmReady && event.target.matches('.segmented-control > .segment') &&
      ['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
    const options = [...event.target.parentElement.querySelectorAll('.segment:not(:disabled)')];
    const current = options.indexOf(event.target);
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
      : (current + (event.key === 'ArrowRight' ? 1 : -1) + options.length) % options.length;
    event.preventDefault();
    options[index]?.focus({ preventScroll: true });
    options[index]?.click();
    return;
  }
  if (!wasmReady || event.isComposing || event.keyCode === 229) return;
  const tagResults = document.getElementById('tag-search-results');
  if ((event.target.id === 'tag-query' || event.target.closest('.tag-search-results')) &&
      ['ArrowDown', 'ArrowUp'].includes(event.key) && tagResults) {
    const options = [...tagResults.querySelectorAll('button')];
    const current = options.indexOf(document.activeElement);
    const index = current < 0 ? (event.key === 'ArrowDown' ? 0 : options.length - 1)
      : (current + (event.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length;
    options[index]?.focus({ preventScroll: true });
    event.preventDefault();
    return;
  }
  if (event.key === 'Escape' && marqueeDrag) { finishMarquee(false); event.preventDefault(); return; }
  if (event.key === 'Escape' && pointerDrag) { finishPointerDrag(); event.preventDefault(); return; }
  const context=event.target.closest('.game-context-menu');
  if(context && ['ArrowDown','ArrowUp','Home','End'].includes(event.key)) {
    const submenu=event.target.closest('.context-submenu');
    const nodes=submenu?[...submenu.querySelectorAll('button')]:[...context.querySelectorAll(':scope>button,:scope>.context-state>button')];
    const current=nodes.indexOf(document.activeElement);
    const index=event.key==='Home'?0:event.key==='End'?nodes.length-1:(current+(event.key==='ArrowDown'?1:-1)+nodes.length)%nodes.length;
    nodes[index]?.focus();event.preventDefault();return;
  }
  if(context && event.key==='ArrowRight' && event.target.closest('.context-state')){context.querySelector('.context-submenu button')?.focus();event.preventDefault();return;}
  if(context && event.key==='ArrowLeft' && event.target.closest('.context-submenu')){context.querySelector('.context-state>button')?.focus();event.preventDefault();return;}
  const choice = event.target.closest('.choice-popup,.choice-trigger');
  if (choice && ui.choice_key(choice.dataset.choiceAnchor || choice.id, event.key)) { event.preventDefault(); return; }
  if (event.key === 'Escape') {
    const action = document.querySelector('.setup-overlay') ? 'setup-defer-all' : document.querySelector('.ai-summary-dialog') ? 'summary-close' : document.querySelector('.library-overlay') ? 'library-close' : document.querySelector('.restore-dialog') ? 'close-history' : document.querySelector('.editor-overlay') ? 'editor-close' : document.querySelector('.game-context-menu') ? 'context-close' : document.querySelector('.reference-detail.editing') ? 'detail-cancel' : 'close-detail';
    if (action === 'close-detail') closeDetail();
    else ui.dispatch(action, '');
    event.preventDefault();
  }
  if (event.key === 'Enter' && event.target.id === 'folder-name') { event.preventDefault(); ui.dispatch('folder-save', ''); }
  if (event.key === 'Enter' && event.target.dataset.input?.startsWith('setup-query:')) { event.preventDefault(); ui.dispatch('setup-search', event.target.dataset.input.slice('setup-query:'.length)); }
  if (event.key === 'Enter' && event.target.id === 'metadata-query') { event.preventDefault(); document.querySelector('[data-action="metadata-search"]:not(:disabled)')?.click(); }
  if (event.key === 'Enter' && event.target.id === 'cover-query') { event.preventDefault(); document.querySelector('[data-action="cover-search"]:not(:disabled)')?.click(); }
  if (document.querySelector('.setup-overlay') && (event.ctrlKey || event.metaKey) && ['s','Enter'].includes(event.key)) { event.preventDefault(); ui.dispatch('setup-save',''); return; }
  if ((event.ctrlKey || event.metaKey) && ['s','Enter'].includes(event.key) && document.querySelector('.reference-detail.editing')) { event.preventDefault(); ui.dispatch('detail-save',''); }
  if (event.key === 'Enter' && event.target.id === 'library-path') ui.dispatch('add-folder', '');
  if (event.key === 'Tab' && document.querySelector('.setup-dialog,.editor-dialog,.library-dialog,.detail-panel')) {
    const modal = document.querySelector('.setup-dialog') || document.querySelector('.editor-dialog') || document.querySelector('.library-dialog') || document.querySelector('.detail-panel');
    const nodes = [...modal.querySelectorAll('button:not(:disabled), input, textarea, select, summary')].filter(e => !e.disabled && e.getClientRects().length);
    const first = nodes[0], last = nodes[nodes.length - 1];
    if (event.shiftKey && document.activeElement === first) { last.focus(); event.preventDefault(); }
    else if (!event.shiftKey && document.activeElement === last) { first.focus(); event.preventDefault(); }
  }
});

window.addEventListener('resize', placeChoice);
window.addEventListener('resize', measureLibraryBounds);
window.addEventListener('resize', () => syncSegmentedControls(false));
document.fonts.ready.then(measureLibraryBounds);
document.fonts.ready.then(() => syncSegmentedControls(false));

function markDropTarget(target) {
  document.querySelectorAll('.drop-target').forEach(e => e.classList.remove('drop-target'));
  if (!target) { clearTimeout(dragHoverTimer); dragHoverKey = null; return; }
  target.classList.add('drop-target');
  const key = target.dataset.dropFolder || 'root';
  if (key !== dragHoverKey) {
    clearTimeout(dragHoverTimer); dragHoverKey = key;
    dragHoverTimer = setTimeout(() => ui.dispatch('folder-hover', target.dataset.dropFolder || ''), 550);
  }
}
function finishPointerDrag() {
  if (pointerDrag?.source.hasPointerCapture(pointerDrag.id)) pointerDrag.source.releasePointerCapture(pointerDrag.id);
  pointerDrag = null;
  dragPreview?.remove(); dragPreview = null;
  clearTimeout(dragHoverTimer); dragHoverKey = null; dragPayload = '';
  document.querySelectorAll('.drop-target').forEach(e => e.classList.remove('drop-target'));
  if (deferredDragFrame) { const frame = deferredDragFrame; deferredDragFrame = null; window.GalGearBridge.render(frame); }
}
document.addEventListener('contextmenu', event => {
  const card=event.target.closest('[data-game-id]');
  if (!wasmReady || !card || document.querySelector('.library-overlay,.detail-overlay')) return;
  event.preventDefault();finishPointerDrag();ui.game_context(card.dataset.gameId,event.clientX,event.clientY);
});
document.addEventListener('keydown',event=> {
  const card=event.target.closest('[data-game-id]');
  if (wasmReady && card && (event.key==='ContextMenu' || event.shiftKey && event.key==='F10')) {
    event.preventDefault();const box=card.getBoundingClientRect();ui.game_context(card.dataset.gameId,box.left+20,box.top+20);
  }
});

function paintMarquee() {
  const drag=marqueeDrag;if (!drag || !drag.active) return;
  const pane=drag.pane;const paneBox=pane.getBoundingClientRect();
  if (drag.y>paneBox.bottom-28) pane.scrollTop+=12;
  else if (drag.y<paneBox.top+28) pane.scrollTop-=12;
  const currentY=drag.y+pane.scrollTop;
  const left=Math.min(drag.startX,drag.x),right=Math.max(drag.startX,drag.x);
  const top=Math.min(drag.startY,currentY),bottom=Math.max(drag.startY,currentY);
  Object.assign(drag.box.style,{left:`${left}px`,top:`${top-pane.scrollTop}px`,width:`${right-left}px`,height:`${bottom-top}px`});
  drag.ids=new Set(drag.initial);
  for(const card of document.querySelectorAll('[data-game-id]')) {
    const bounds=card.getBoundingClientRect();
    const intersects=bounds.left<right && bounds.right>left && bounds.top+pane.scrollTop<bottom && bounds.bottom+pane.scrollTop>top;
    if(intersects) drag.ids.add(card.dataset.gameId);
    const selected=drag.ids.has(card.dataset.gameId);
    card.classList.toggle('is-selected',selected);
    card.querySelector('.game-face')?.setAttribute('aria-pressed',String(selected));
    const check=card.querySelector('.selection-check');
    if(check) check.innerHTML=selected ? '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 12 4 4L19 6"/></svg>' : '';
  }
  marqueeFrame=requestAnimationFrame(paintMarquee);
}
function finishMarquee(commit=true) {
  if(!marqueeDrag)return;
  if(commit && marqueeDrag.active) {
    cancelAnimationFrame(marqueeFrame);
    paintMarquee();
  }
  const drag=marqueeDrag;marqueeDrag=null;cancelAnimationFrame(marqueeFrame);marqueeFrame=null;drag.box?.remove();
  if(drag.pane.hasPointerCapture(drag.id))drag.pane.releasePointerCapture(drag.id);
  if(drag.active) {
    // The committed Rust paint already includes updates received during the drag.
    deferredDragFrame=null;
    suppressCardClickUntil=performance.now()+400;
    if(commit) {
      ui.library_marquee(JSON.stringify([...drag.ids]),false);
      if(drag.ids.size===0) ui.dispatch('batch-toggle','');
    }
    else {
      ui.library_marquee(JSON.stringify(drag.original),false);
      if(!drag.wasSelecting) ui.dispatch('batch-toggle','');
    }
  }
  if(deferredDragFrame){const html=deferredDragFrame;deferredDragFrame=null;window.GalGearBridge.render(html);}
}
document.addEventListener('pointerdown', event => {
  if (pointerDrag || marqueeDrag || event.isPrimary === false) return;
  const source = event.target.closest('[data-game-id]');
  const grid=event.target.closest('.game-grid');
  if(wasmReady && event.button===0 && !event.altKey && grid && !document.querySelector('.library-bar [data-action=batch-toggle]:disabled') && !event.target.closest('.card-action,.card-remove') && !document.querySelector('.library-overlay,.detail-overlay,.game-context-menu')) {
    const pane=document.getElementById('page-scroll');
    if(!pane) return;
    const bounds=grid.getBoundingClientRect();
    const original=[...document.querySelectorAll('.game-card.is-selected')].map(card=>card.dataset.gameId);
    marqueeDrag={id:event.pointerId,pane,grid,offsetX:event.clientX-bounds.left,offsetY:event.clientY-bounds.top,startX:event.clientX,startY:event.clientY+pane.scrollTop,x:event.clientX,y:event.clientY,active:false,wasSelecting:!!document.querySelector('.library-bar.selecting'),original,initial:event.ctrlKey||event.shiftKey?original:[],ids:new Set()};
    return;
  }
  if (!wasmReady || event.button !== 0 || !source || event.target.closest('.card-action,.card-remove') || document.querySelector('.library-overlay,.detail-overlay')) return;
  pointerDrag = { source, id: event.pointerId, x: event.clientX, y: event.clientY };
});
document.addEventListener('pointermove', event => {
  if(marqueeDrag && marqueeDrag.id===event.pointerId) {
    const drag=marqueeDrag;drag.x=event.clientX;drag.y=event.clientY;
    if(!drag.active && Math.hypot(drag.x-drag.startX,drag.y+drag.pane.scrollTop-drag.startY)>=6) {
      if(!drag.wasSelecting) {
        // Render batch controls before capturing the pointer and painting the rectangle.
        marqueeDrag=null;
        ui.dispatch('batch-toggle','');
        marqueeDrag=drag;
        if(!document.querySelector('.library-bar.selecting')) { finishMarquee(false); return; }
        drag.pane=document.getElementById('page-scroll');
        const bounds=drag.grid.getBoundingClientRect();
        drag.startX=bounds.left+drag.offsetX;
        drag.startY=bounds.top+drag.offsetY+drag.pane.scrollTop;
      }
      drag.active=true;drag.pane.setPointerCapture(event.pointerId);
      drag.box=document.createElement('div');drag.box.className='marquee-box';drag.box.setAttribute('aria-hidden','true');document.body.append(drag.box);paintMarquee();
    }
    if(drag.active)event.preventDefault();return;
  }
  if (!pointerDrag || pointerDrag.id !== event.pointerId) return;
  if (!dragPayload) {
    if (Math.hypot(event.clientX - pointerDrag.x, event.clientY - pointerDrag.y) < 8) return;
    dragPayload = ui.library_drag_ids(pointerDrag.source.dataset.gameId);
    if (!dragPayload) { finishPointerDrag(); return; }
    pointerDrag.source.setPointerCapture(event.pointerId);
    dragPreview = pointerDrag.source.cloneNode(true);
    dragPreview.removeAttribute('id'); dragPreview.querySelectorAll('[id]').forEach(e => e.removeAttribute('id'));
    dragPreview.classList.add('drag-preview'); dragPreview.setAttribute('aria-hidden', 'true'); dragPreview.inert = true;
    document.body.append(dragPreview);
  }
  event.preventDefault();
  dragPreview.style.left = `${event.clientX + 14}px`;
  dragPreview.style.top = `${event.clientY + 14}px`;
  const under = document.elementFromPoint(event.clientX, event.clientY);
  markDropTarget(under?.closest('[data-drop-folder],[data-drop-expand]'));
  const tree = under?.closest('.collection-tree');
  if (tree) {
    const box = tree.getBoundingClientRect();
    if (event.clientY < box.top + 24) tree.scrollTop -= 18;
    else if (event.clientY > box.bottom - 24) tree.scrollTop += 18;
  }
});
document.addEventListener('pointerup', event => {
  if(marqueeDrag && marqueeDrag.id===event.pointerId){finishMarquee(true);return;}
  if (!pointerDrag || pointerDrag.id !== event.pointerId) return;
  if (dragPayload) {
    const target = document.elementFromPoint(event.clientX, event.clientY)?.closest('[data-drop-folder]');
    if (target) ui.library_drop(target.dataset.dropFolder, dragPayload);
    suppressCardClickUntil = performance.now() + 400;
    event.preventDefault();
  }
  finishPointerDrag();
});
document.addEventListener('pointercancel', event=>{
  if(pointerDrag?.id===event.pointerId) finishPointerDrag();
  if(marqueeDrag?.id===event.pointerId) finishMarquee(false);
});

document.addEventListener('dragstart', event => {
  const card = event.target.closest('[data-game-id]');
  if (!wasmReady || !card || !event.dataTransfer) return;
  if (pointerDrag || marqueeDrag) { event.preventDefault(); return; }
  dragPayload = ui.library_drag_ids(card.dataset.gameId);
  event.dataTransfer.setData('application/x-galgear-games', dragPayload);
  event.dataTransfer.setData('text/plain', dragPayload);
  event.dataTransfer.effectAllowed = 'copy';
});
document.addEventListener('dragover', event => {
  if (!dragPayload || !event.dataTransfer) return;
  const target = event.target.closest('[data-drop-folder],[data-drop-expand]');
  if (!target) return;
  event.preventDefault(); event.dataTransfer.dropEffect = 'copy';
  document.querySelectorAll('.drop-target').forEach(e => e.classList.remove('drop-target'));
  target.classList.add('drop-target');
  const key = target.dataset.dropFolder || 'root';
  if (key !== dragHoverKey) {
    clearTimeout(dragHoverTimer); dragHoverKey = key;
    dragHoverTimer = setTimeout(() => ui.dispatch('folder-hover', target.dataset.dropFolder || ''), 550);
  }
});
document.addEventListener('drop', event => {
  const target = event.target.closest('[data-drop-folder]');
  if (wasmReady && target && dragPayload) {
    // WebView2 can expose a protected/empty custom MIME payload during native drops.
    // The active internal gesture already owns the serialized Rust selection.
    event.preventDefault(); ui.library_drop(target.dataset.dropFolder, dragPayload);
  }
  clearTimeout(dragHoverTimer); dragHoverKey = null; dragPayload = '';
  document.querySelectorAll('.drop-target').forEach(e => e.classList.remove('drop-target'));
  if (deferredDragFrame) { const frame = deferredDragFrame; deferredDragFrame = null; window.GalGearBridge.render(frame); }
});
document.addEventListener('dragend', () => {
  clearTimeout(dragHoverTimer); dragHoverKey = null; dragPayload = '';
  document.querySelectorAll('.drop-target').forEach(e => e.classList.remove('drop-target'));
  if (deferredDragFrame) { const frame = deferredDragFrame; deferredDragFrame = null; window.GalGearBridge.render(frame); }
});
document.addEventListener('scroll', event => {
  if (wasmReady && !restoringScroll && event.target.isConnected && document.querySelector('.choice-popup') && event.target.matches?.('.page-scroll,.detail-scroll,.setup-table-scroll')) ui.dispatch('choice-close', '');
}, true);

try {
  await init(); wasmReady = true;
  if (window.__TAURI__?.event) {
    await window.__TAURI__.event.listen('scan-progress', event => ui.scan_progress(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('game-enriched', event => ui.game_enriched(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('enrichment-progress', event => ui.enrichment_progress(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('fan-download-progress', event => ui.download_progress(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('fan-status-updated', event => ui.fan_status_updated(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('runtime-updated', event => ui.runtime_updated(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('playtime-updated', event => ui.playtime_updated(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('backup-completed', event => ui.backup_completed(JSON.stringify(event.payload)));
    await window.__TAURI__.event.listen('tool-failed', event => ui.tool_failed(JSON.stringify(event.payload)));
  }
}
catch (error) {
  const screen = document.createElement('div');
  screen.className = 'boot-screen';
  screen.textContent = String(error);
  document.getElementById('app').replaceChildren(screen);
}
