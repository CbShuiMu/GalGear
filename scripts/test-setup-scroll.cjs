const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {test} = require('node:test');
const vm = require('node:vm');

const source = readFileSync('web/setup-scroll.js', 'utf8')
  .replace(/^import .*\n/, '').replaceAll('export function', 'function');

test('reused spacers update height without writing HTML style attributes', () => {
  const context = vm.createContext({});
  vm.runInContext(source, context);
  // CSP 会拦截 setAttribute('style', ...)，复用首部占位行时尤其容易留下旧高度。
  const leading = {
    dataset: {setupSpacerRows: '16'}, style: {},
    setAttribute() { assert.fail('spacer heights must use CSSOM'); },
  };
  const trailing = {
    dataset: {setupSpacerRows: '117'}, style: {},
    setAttribute() { assert.fail('spacer heights must use CSSOM'); },
  };
  const scroller = {querySelectorAll: () => [leading, trailing]};
  for (const [start, end] of [[16, 117], [30, 103], [77, 56], [133, 0], [0, 133]]) {
    leading.dataset.setupSpacerRows = String(start);
    trailing.dataset.setupSpacerRows = String(end);
    context.syncSetupSpacers(scroller);
    assert.equal(leading.style.height, `calc(var(--setup-row-height) * ${start})`);
    assert.equal(trailing.style.height, `calc(var(--setup-row-height) * ${end})`);
  }
  context.syncSetupSpacers(null);
});

function fixture() {
  const frames = [];
  const listeners = {};
  const updates = [];
  const body = {
    dataset: {setupTotal: '600', setupStart: '0', setupCount: '32', setupPinned: ''},
    querySelector: selector => selector === '[data-setup-index]' ? row : null,
  };
  const scroll = {
    scrollTop: 0, clientHeight: 460,
    querySelector: selector => selector === 'tbody' ? body : {},
    addEventListener: (name, callback) => { listeners[name] = callback; },
    removeEventListener() {},
  };
  const row = {
    dataset: {setupIndex: '0', setupGame: 'first'}, parentElement: body,
    closest: () => scroll,
    getBoundingClientRect: () => ({height: 115}),
  };
  const control = {closest: () => row};
  const nextControl = {};
  const document = {
    activeElement: control,
    getElementById: () => scroll,
    querySelector: selector => selector.startsWith('[data-setup-index=') ? {next: true} : null,
    addEventListener: (name, callback) => { listeners[name] = callback; },
  };
  const context = vm.createContext({
    document,
    ResizeObserver: class {
      constructor(callback) { listeners.resize = callback; }
      observe() {}
      disconnect() {}
    },
    requestAnimationFrame: callback => { frames.push(callback); return frames.length; },
    focusable: node => node?.next ? [nextControl] : [control],
    focusNode: node => { document.activeElement = node; },
  });
  vm.runInContext(source, context);
  const schedule = context.installSetupScroll((start, count, pinned) => {
    updates.push({start, count, pinned, scrollTop: scroll.scrollTop});
    Object.assign(body.dataset, {setupStart: String(start), setupCount: String(count), setupPinned: pinned});
  });
  const flush = () => { const callbacks = frames.splice(0); callbacks.forEach(callback => callback()); };
  return {schedule, flush, frames, listeners, updates, scroll, document, control, nextControl};
}

test('scroll bursts coalesce and small movements keep the current rendered window', () => {
  const app = fixture();
  app.schedule(); app.flush();
  app.updates.length = 0;
  for (let top = 1; top <= 100; top++) {
    app.scroll.scrollTop = top;
    app.listeners.scroll();
  }
  assert.equal(app.frames.length, 1);
  app.flush();
  assert.equal(app.updates.length, 0);
  app.scroll.scrollTop = 68_500;
  app.listeners.scroll(); app.flush();
  assert.equal(app.updates.length, 1);
  const update = app.updates[0];
  assert.equal(update.pinned, 'first');
  assert.ok(update.start <= 595 && update.start + update.count >= 600);
  assert.ok(update.count <= 32);
});

test('Tab from a pinned offscreen row moves the viewport before rendering its neighbour', () => {
  const app = fixture();
  app.schedule(); app.flush();
  app.scroll.scrollTop = 68_500;
  app.listeners.scroll(); app.flush();
  const event = {
    key: 'Tab', target: app.control,
    preventDefault() { this.prevented = true; },
    stopImmediatePropagation() {},
  };
  app.listeners.keydown(event);
  assert.equal(event.prevented, true);
  assert.equal(app.updates.at(-1).scrollTop, 115);
  assert.equal(app.updates.at(-1).start, 0);
  app.flush();
  assert.equal(app.document.activeElement, app.nextControl);
});

test('a taller viewport renders the newly visible rows without another scroll event', () => {
  const app = fixture();
  app.schedule(); app.flush();
  app.scroll.scrollTop = 11_500;
  app.listeners.scroll(); app.flush();
  const before = app.updates.at(-1);
  app.scroll.clientHeight = 2_300;
  app.listeners.resize(); app.flush();
  const after = app.updates.at(-1);
  assert.ok(after.count > before.count);
  assert.ok(after.start <= 100 && after.start + after.count >= 120);
  assert.equal(after.scrollTop, before.scrollTop);
  assert.equal(after.pinned, 'first');
});
