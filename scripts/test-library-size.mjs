import assert from 'node:assert/strict';
import {test} from 'node:test';
import {libraryCardLayout, installLibrarySize} from '../web/library-size.js';

test('smaller sizes always add columns across desktop widths, scaling and collection panes', () => {
  for (let viewport = 760; viewport <= 3840; viewport += 7) {
    for (const inset of [144, 160, 378]) {
      const available = viewport - inset;
      const layouts = ['normal', 'small', 'tiny'].map(size => libraryCardLayout(available, viewport, size));
      assert.ok(layouts[0].columns < layouts[1].columns);
      assert.ok(layouts[1].columns < layouts[2].columns);
      assert.ok(layouts[0].width > layouts[1].width && layouts[1].width > layouts[2].width);
      for (const {columns, width} of layouts) {
        assert.ok(columns * width + (columns - 1) * 20 <= available + .5);
      }
    }
  }
});

test('reference window has 4/5/6 columns and fullscreen grows compact covers only slightly', () => {
  assert.deepEqual(['normal', 'small', 'tiny'].map(size => libraryCardLayout(1098, 1242, size).columns), [4,5,6]);
  const large = ['normal', 'small', 'tiny'].map(size => libraryCardLayout(1776, 1920, size));
  assert.deepEqual(large.map(l => l.columns), [5,8,10]);
  assert.ok(large[1].width <= 205 && large[2].width <= 166);
});

test('Alt input respects editing, overlays, drag state, key repeats and wheel throttling', () => {
  const handlers = {}, steps = [];
  let editing = false, modal = false, dragging = false, inert = false;
  const page = {closest: selector => selector === '[inert]' ? inert : null, contains: node => node === target};
  const target = {closest: selector => selector === '.library-page' ? page : editing};
  globalThis.document = {
    querySelector: selector => selector === '.library-page' ? page : modal,
    addEventListener: (name, handler) => { handlers[name] = handler; },
  };
  installLibrarySize({step: value => steps.push(value), isDragging: () => dragging});
  const fire = (name, overrides = {}) => {
    let prevented = false;
    handlers[name]({target, altKey: true, key: 'ArrowDown', deltaY: 120,
      preventDefault() { prevented = true; }, stopImmediatePropagation() {}, ...overrides});
    return prevented;
  };
  assert.ok(fire('keydown'));
  assert.ok(fire('keydown', {key: 'ArrowUp'}));
  fire('keydown', {repeat: true});
  for (const overrides of [{altKey: false}, {ctrlKey: true}, {shiftKey: true}, {isComposing: true}, {key: 'ArrowLeft'}]) {
    assert.equal(fire('keydown', overrides), false);
  }
  editing = true; assert.equal(fire('keydown'), false); editing = false;
  modal = true; assert.equal(fire('wheel'), false); modal = false;
  dragging = true; assert.equal(fire('keydown'), false); dragging = false;
  inert = true; assert.equal(fire('keydown'), false); inert = false;
  assert.ok(fire('wheel')); assert.ok(fire('wheel'));
  assert.deepEqual(steps, ['smaller', 'larger', 'smaller']);
  delete globalThis.document;
});

test('Alt wheel keeps working over empty scroll-pane space after fullscreen cards shrink', t => {
  const handlers = {}, steps = [];
  const target = () => ({closest: () => null});
  const card = target(), blank = target(), sidebar = target(), otherPane = target();
  const pane = target();
  pane.contains = node => [pane, card, blank].includes(node);
  let libraryVisible = true;
  const page = {
    closest: selector => selector === '.page-scroll' ? pane : null,
    contains: node => node === card,
  };
  card.closest = selector => selector === '.library-page' ? page : null;
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const previousDocument = globalThis.document;
  globalThis.document = {
    querySelector: selector => selector === '.library-page' && libraryVisible ? page : null,
    addEventListener: (name, handler) => { handlers[name] = handler; },
  };
  t.after(() => {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
  });
  installLibrarySize({step: direction => steps.push(direction), isDragging: () => false});
  const wheel = (target, deltaY = 120, altKey = true) => {
    now += 250;
    let prevented = false;
    handlers.wheel({target, deltaY, altKey, preventDefault() { prevented = true; }});
    return prevented;
  };
  // The pointer starts on a second row, then falls below the shorter page after shrinking.
  assert.equal(wheel(card), true);
  assert.equal(wheel(pane), true);
  assert.equal(wheel(blank, -120), true);
  assert.deepEqual(steps, ['smaller', 'smaller', 'larger']);
  assert.equal(wheel(blank, 120, false), false);
  assert.equal(wheel(sidebar), false);
  assert.equal(wheel(otherPane), false);
  libraryVisible = false;
  assert.equal(wheel(pane), false);
  assert.equal(steps.length, 3);
});
