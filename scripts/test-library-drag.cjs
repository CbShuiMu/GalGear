const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

// Exercise the bridge's actual event handlers without requiring a desktop session.
const bridge = fs.readFileSync(path.join(__dirname, '../web/bridge.js'), 'utf8');
const keyboard = bridge.slice(bridge.indexOf("document.addEventListener('keydown', event => {"), bridge.indexOf("window.addEventListener('resize', placeChoice);"));
const gestures = bridge.slice(bridge.indexOf('function markDropTarget('), bridge.indexOf("document.addEventListener('dragover', event => {"));

function fixture({ batch = false, selected = [], collections = false, selectedFolders = [] } = {}) {
  const listeners = new Map();
  const calls = [];
  const selection = { rangeCount: 1, removeAllRanges() { this.rangeCount = 0; } };
  const frames = new Map();
  let frameId = 0;
  const classes = values => {
    const names = new Set(values);
    return {
      add: name => names.add(name),
      remove: name => names.delete(name),
      contains: name => names.has(name),
      toggle(name, enabled) { enabled ? names.add(name) : names.delete(name); },
    };
  };
  const element = (bounds = {}) => {
    const capture = new Set();
    return {
      dataset: {}, style: {}, classList: classes([]), scrollTop: 0,
      getBoundingClientRect: () => ({ left: 0, top: 0, right: 400, bottom: 800, ...bounds }),
      setAttribute() {}, removeAttribute() {}, remove() {},
      setPointerCapture: id => capture.add(id),
      releasePointerCapture: id => capture.delete(id),
      hasPointerCapture: id => capture.has(id),
      querySelectorAll: () => [],
      closest: () => null,
      matches: () => false,
    };
  };
  const pane = element();
  const grid = element({ top: collections ? 300 : 100 });
  const folderGrid = element({ top: 100 });
  const cards = ['a', 'b'].map((id, index) => {
    const card = element({ left: index * 120, top: collections ? 300 : 100, right: index * 120 + 100, bottom: collections ? 440 : 240 });
    card.dataset.gameId = id;
    card.classList = classes(selected.includes(id) ? ['is-selected'] : []);
    card.querySelector = selector => selector === '.selection-check' && !batch ? null : element();
    card.closest = selector => selector === '[data-game-id]' ? card : selector === '.game-grid,.folder-grid' ? grid : null;
    card.cloneNode = () => element();
    return card;
  });
  const folders = collections ? ['a', 'folder-b'].map((id, index) => {
    const card = element({ left: index * 120, top: 100, right: index * 120 + 100, bottom: 240 });
    card.dataset.folderId = id;
    card.classList = classes(selectedFolders.includes(id) ? ['is-selected'] : []);
    card.querySelector = selector => selector === '.selection-check' && !batch ? null : element();
    card.closest = selector => selector === '.game-grid,.folder-grid' ? folderGrid : null;
    return card;
  }) : [];
  const scope = { querySelectorAll: () => [...folders, ...cards] };
  grid.closest = folderGrid.closest = selector => selector === '.library-page' ? scope : null;
  const folder = element();
  folder.dataset.dropFolder = 'folder';
  folder.closest = selector => selector.includes('[data-drop-folder]') ? folder : null;
  const blank = element();
  blank.closest = selector => selector === '.game-grid,.folder-grid' ? grid : null;
  const folderBlank = element();
  folderBlank.closest = selector => selector === '.game-grid,.folder-grid' ? folderGrid : null;
  const document = {
    body: { append() {} },
    addEventListener(name, handler) {
      if (!listeners.has(name)) listeners.set(name, []);
      listeners.get(name).push(handler);
    },
    getElementById: id => id === 'page-scroll' ? pane : null,
    getSelection: () => selection,
    createElement: () => element(),
    elementFromPoint: () => folder,
    querySelector: selector => selector === '.library-bar.selecting' && batch ? element() : null,
    querySelectorAll(selector) {
      if (selector === '[data-game-id]') return cards;
      if (selector === '.game-card.is-selected') return cards.filter(card => card.classList.contains('is-selected'));
      return [];
    },
  };
  const ui = {
    dispatch(action) {
      calls.push(['dispatch', action]);
      if (action === 'batch-toggle') batch = !batch;
    },
    library_marquee(payload) {
      const selection = JSON.parse(payload);
      calls.push(collections ? ['selection', selection.games, selection.folders] : ['selection', selection.games]);
      cards.forEach(card => card.classList.toggle('is-selected', selection.games.includes(card.dataset.gameId)));
      folders.forEach(card => card.classList.toggle('is-selected', selection.folders.includes(card.dataset.folderId)));
    },
    library_drag_ids(id) { calls.push(['drag', id]); return JSON.stringify([id]); },
    library_drop(id, payload) { calls.push(['drop', id, JSON.parse(payload)]); },
  };
  const context = vm.createContext({
    document, ui, Math, JSON, Set, String, performance: { now: () => 100 },
    window: { addEventListener: document.addEventListener, GalGearBridge: { render() {} } },
    setTimeout: () => 1, clearTimeout() {},
    requestAnimationFrame(callback) { frames.set(++frameId, callback); return frameId; },
    cancelAnimationFrame(id) { frames.delete(id); },
  });
  vm.runInContext(`
    let wasmReady = true, pointerDrag = null, marqueeDrag = null, marqueeFrame = null;
    let dragPreview = null, dragPayload = '', dragHoverTimer = null, dragHoverKey = null;
    let deferredDragFrame = null, suppressCardClickUntil = 0;
    ${keyboard}
    ${gestures}
  `, context);
  const send = (name, fields = {}) => {
    const event = {
      target: cards[0], button: 0, pointerId: 1, isPrimary: true,
      clientX: 20, clientY: 130, key: '', altKey: false,
      preventDefault() { this.prevented = true; },
      ...fields,
    };
    for (const listener of listeners.get(name) || []) listener(event);
    return event;
  };
  return {
    cards, folders, blank, folderBlank, pane, send, calls, selection,
    batch: () => batch,
    state: expression => vm.runInContext(expression, context),
    frame() {
      const pending = [...frames.values()]; frames.clear();
      for (const callback of pending) callback();
    },
  };
}

test('a plain click or movement below the threshold keeps normal card behavior', () => {
  const app = fixture();
  app.send('pointerdown');
  app.send('pointermove', { clientX: 23, clientY: 133 });
  app.send('pointerup');
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, []);
  assert.equal(app.state('suppressCardClickUntil'), 0);
});

test('native text selection is blocked from press through release, including outside the grid', () => {
  const app = fixture();
  const outside = { closest: () => null };
  const down = app.send('pointerdown', { target: app.blank });
  assert.equal(down.prevented, undefined, 'a press still allows normal focus and click');
  assert.equal(app.selection.rangeCount, 0, 'clear a stale selection before any batch render');
  app.send('pointermove', { clientX: 23, clientY: 133 });
  assert.equal(app.send('selectstart', { target: outside }).prevented, true);
  app.send('pointermove', { clientX: 180, clientY: 200 });
  assert.equal(app.send('selectstart', { target: outside }).prevented, true);
  app.selection.rangeCount = 1;
  app.send('pointerup');
  assert.equal(app.selection.rangeCount, 0, 'no native Range survives into a subsequent detail render');
  assert.equal(app.send('selectstart', { target: outside }).prevented, undefined);
});

test('selection suppression ends after a click, Escape, pointer cancellation, or loss of window focus', () => {
  for (const end of ['pointerup', 'keydown', 'pointercancel', 'blur']) {
    const app = fixture();
    app.send('pointerdown');
    assert.equal(app.send('selectstart').prevented, true);
    app.send(end, { key: 'Escape' });
    assert.equal(app.state('marqueeDrag'), null, end);
    assert.equal(app.send('selectstart').prevented, undefined, end);
  }
});

test('losing focus cancels an active marquee and releases the pointer and native selection', () => {
  const app = fixture();
  app.send('pointerdown');
  app.send('pointermove', { clientX: 180, clientY: 200 });
  app.selection.rangeCount = 1;
  app.send('blur');
  assert.equal(app.batch(), false);
  assert.equal(app.pane.hasPointerCapture(1), false);
  assert.equal(app.selection.rangeCount, 0);
  assert.equal(app.send('selectstart').prevented, undefined);
});

test('Alt dragging also prevents native text selection and clears it on release', () => {
  const app = fixture();
  app.send('pointerdown', { altKey: true });
  assert.equal(app.selection.rangeCount, 0);
  assert.equal(app.send('selectstart').prevented, true);
  app.send('pointermove', { clientX: 180, clientY: 200 });
  app.selection.rangeCount = 1;
  app.send('pointerup');
  assert.equal(app.selection.rangeCount, 0);
  assert.equal(app.send('selectstart').prevented, undefined);
});

test('ordinary detail text and editable fields retain native selection', () => {
  const app = fixture();
  const target = { closest: () => null };
  app.send('pointerdown', { target });
  assert.equal(app.send('selectstart', { target }).prevented, undefined);
  assert.equal(app.selection.rangeCount, 1);
  app.send('pointerup', { target });
  assert.equal(app.selection.rangeCount, 1);
});

test('dragging from a card enters batch mode and commits the intersected cards', () => {
  const app = fixture();
  app.send('pointerdown');
  app.send('pointermove', { clientX: 180, clientY: 200 });
  assert.equal(app.batch(), true);
  assert.equal(app.pane.hasPointerCapture(1), true);
  app.send('pointerup', { clientX: 180, clientY: 200 });
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', ['a', 'b']]]);
  assert.equal(app.pane.hasPointerCapture(1), false);
  assert.ok(app.state('suppressCardClickUntil') > 100);
});

test('dragging from empty grid space also starts batch selection', () => {
  const app = fixture();
  app.send('pointerdown', { target: app.blank, clientX: 300 });
  app.send('pointermove', { target: app.blank, clientX: 180, clientY: 200 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', ['b']]]);
});

test('releasing an empty marquee exits automatically entered batch mode', () => {
  const app = fixture();
  app.send('pointerdown', { target: app.blank, clientX: 300, clientY: 300 });
  app.send('pointermove', { target: app.blank, clientX: 350, clientY: 350 });
  app.send('pointerup');
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', []], ['dispatch', 'batch-toggle']]);
});

test('releasing an empty marquee also exits pre-existing batch mode', () => {
  const app = fixture({ batch: true, selected: ['b'] });
  app.send('pointerdown', { target: app.blank, clientX: 300, clientY: 300 });
  app.send('pointermove', { target: app.blank, clientX: 350, clientY: 350 });
  app.send('pointerup');
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, [['selection', []], ['dispatch', 'batch-toggle']]);
});

test('empty modifier dragging keeps the retained selection in batch mode', () => {
  const app = fixture({ batch: true, selected: ['b'] });
  app.send('pointerdown', { target: app.blank, clientX: 300, clientY: 300, ctrlKey: true });
  app.send('pointermove', { target: app.blank, clientX: 350, clientY: 350 });
  app.send('pointerup');
  assert.equal(app.batch(), true);
  assert.deepEqual(app.calls, [['selection', ['b']]]);
});

test('a click without dragging leaves empty pre-existing batch mode unchanged', () => {
  const app = fixture({ batch: true });
  app.send('pointerdown', { target: app.blank });
  app.send('pointerup', { target: app.blank });
  assert.equal(app.batch(), true);
  assert.deepEqual(app.calls, []);
});

test('release uses the final pointer move even before the next animation frame', () => {
  const app = fixture();
  app.send('pointerdown', { target: app.blank, clientX: 300, clientY: 300 });
  app.send('pointermove', { clientX: 180, clientY: 200 });
  app.send('pointermove', { clientX: 350, clientY: 350 });
  app.send('pointerup');
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', []], ['dispatch', 'batch-toggle']]);
});

test('Escape rolls back automatic batch entry and releases pointer capture', () => {
  const app = fixture();
  app.send('pointerdown');
  app.send('pointermove', { clientX: 180, clientY: 200 });
  const escape = app.send('keydown', { key: 'Escape' });
  assert.equal(escape.prevented, true);
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', []], ['dispatch', 'batch-toggle']]);
  assert.equal(app.pane.hasPointerCapture(1), false);
});

test('Escape restores an existing selection while keeping existing batch mode', () => {
  const app = fixture({ batch: true, selected: ['b'] });
  app.send('pointerdown');
  app.send('pointermove', { clientX: 70, clientY: 200 });
  app.send('keydown', { key: 'Escape' });
  assert.equal(app.batch(), true);
  assert.deepEqual(app.calls, [['selection', ['b']]]);
});

test('cancelling an empty marquee preserves pre-existing empty batch mode', () => {
  const app = fixture({ batch: true });
  app.send('pointerdown', { target: app.blank, clientX: 300, clientY: 300 });
  app.send('pointermove', { target: app.blank, clientX: 350, clientY: 350 });
  app.send('pointercancel');
  assert.equal(app.batch(), true);
  assert.deepEqual(app.calls, [['selection', []]]);
});

test('modifier dragging adds cards to the existing batch selection', () => {
  const app = fixture({ batch: true, selected: ['b'] });
  app.send('pointerdown', { ctrlKey: true });
  app.send('pointermove', { clientX: 70, clientY: 200 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['selection', ['b', 'a']]]);
});

test('Alt dragging preserves the collection drop gesture', () => {
  const app = fixture();
  app.send('pointerdown', { altKey: true });
  app.send('pointermove', { clientX: 180, clientY: 200 });
  app.send('pointerup');
  assert.equal(app.batch(), false);
  assert.deepEqual(app.calls, [['drag', 'a'], ['drop', 'folder', ['a']]]);
});

test('touch cancellation rolls back only the pointer that owns the gesture', () => {
  const app = fixture();
  app.send('pointerdown', { pointerType: 'touch' });
  app.send('pointermove', { pointerType: 'touch', clientX: 180, clientY: 200 });
  app.send('pointerdown', { pointerId: 2, isPrimary: false });
  app.send('pointercancel', { pointerId: 2 });
  assert.equal(app.batch(), true);
  assert.equal(app.state('marqueeDrag.id'), 1);
  app.send('pointercancel', { pointerId: 1 });
  assert.equal(app.batch(), false);
  assert.equal(app.state('marqueeDrag'), null);
});

test('dragging collection cards enters batch mode without confusing matching game IDs', () => {
  const app = fixture({ collections: true });
  app.send('pointerdown', { target: app.folders[0] });
  app.send('pointermove', { clientX: 180, clientY: 200 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', [], ['a', 'folder-b']]]);
  assert.equal(app.cards[0].classList.contains('is-selected'), false);
});

test('collection marquee can span folders and games and start in empty folder space', () => {
  const app = fixture({ collections: true });
  app.send('pointerdown', { target: app.folderBlank, clientX: 300 });
  app.send('pointermove', { clientX: 20, clientY: 350 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['dispatch', 'batch-toggle'], ['selection', ['a', 'b'], ['a', 'folder-b']]]);
});

test('collection modifier selection and Escape preserve both kinds of selection', () => {
  const app = fixture({ collections: true, batch: true, selected: ['b'], selectedFolders: ['folder-b'] });
  app.send('pointerdown', { target: app.folders[0], shiftKey: true });
  app.send('pointermove', { clientX: 70, clientY: 200 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['selection', ['b'], ['folder-b', 'a']]]);
  app.send('pointerdown', { target: app.folderBlank, clientX: 300, clientY: 500 });
  app.send('pointermove', { clientX: 350, clientY: 550 });
  app.send('keydown', { key: 'Escape' });
  assert.deepEqual(app.calls.at(-1), ['selection', ['b'], ['a', 'folder-b']]);
  assert.equal(app.batch(), true);
});

test('empty collection marquee clears folders and games and exits batch mode', () => {
  const app = fixture({ collections: true, batch: true, selected: ['a'], selectedFolders: ['a'] });
  app.send('pointerdown', { target: app.folderBlank, clientX: 300, clientY: 500 });
  app.send('pointermove', { clientX: 350, clientY: 550 });
  app.send('pointerup');
  assert.deepEqual(app.calls, [['selection', [], []], ['dispatch', 'batch-toggle']]);
  assert.equal(app.batch(), false);
});
