const assert = require('node:assert/strict');
const {test} = require('node:test');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const source = readFileSync(require('node:path').join(__dirname, '../web/detail-tag-reorder.js'), 'utf8');
const pressSource = readFileSync(require('node:path').join(__dirname, '../web/press-gesture.js'), 'utf8');

function fixture({reduced = false, width = 300} = {}) {
  const listeners = new Map(), timers = new Map(), frames = new Map();
  const animations = [], commits = [], renders = [];
  let sequence = 0, finishSave, now = 1000, reads = 0;
  const rect = (x, y, w, h) => ({x, y, left: x, top: y, right: x + w, bottom: y + h, width: w, height: h});
  class Element {
    constructor(className = '', size = 60) {
      this.className = className; this.size = size;
      this.children = []; this.dataset = {}; this.style = {}; this.scrollTop = 0;
      this.classList = {
        contains: name => this.className.split(' ').includes(name),
        add: name => { this.className += ` ${name}`; },
        remove: name => { this.className = this.className.split(' ').filter(n => n !== name).join(' '); }
      };
    }
    get isConnected() { return !!this.parentElement; }
    contains(node) { return this === node || this.children.some(child => child.contains(node)); }
    get nextElementSibling() { return this.parentElement?.children[this.parentElement.children.indexOf(this) + 1] ?? null; }
    append(child) { this.insertBefore(child, null); }
    insertBefore(child, before) {
      child.remove();
      const index = before ? this.children.indexOf(before) : this.children.length;
      assert.ok(index >= 0);
      this.children.splice(index, 0, child); child.parentElement = this;
    }
    remove() {
      if (!this.parentElement) return;
      const siblings = this.parentElement.children;
      siblings.splice(siblings.indexOf(this), 1); this.parentElement = null;
    }
    setAttribute() {}
    removeAttribute(name) { if (name === 'data-value') delete this.dataset.value; }
    setPointerCapture() { this.captured = true; }
    hasPointerCapture() { return this.captured; }
    releasePointerCapture() { this.captured = false; }
    focus() { document.activeElement = this; }
    blur() { if (document.activeElement === this) document.activeElement = null; }
    closest(selector) {
      if (selector === '.hero-tags .detail-tag-chip:not(.tag-input-chip)') {
        return this.classList.contains('detail-tag-chip') ? this : this.parentElement?.closest(selector) ?? null;
      }
      if (selector === '[data-action]') return this.dataset.action ? this : this.parentElement?.closest(selector) ?? null;
      if (selector.includes('.game-tags')) return list.dataset.tagReorder === 'true' ? list : null;
      if (selector === '.detail-scroll') return pane;
      return shell;
    }
    querySelectorAll() { return this.children.filter(child => child.classList.contains('detail-tag-chip')); }
    querySelector(selector) { return this.children.find(child => child.classList.contains(selector.slice(1))) ?? null; }
    cloneNode() { const copy = new Element(this.className, this.size); copy.dataset = {...this.dataset}; return copy; }
    getBoundingClientRect() {
      reads++;
      if (this === pane) return rect(0, 0, 800, 600);
      if (this === list) return rect(10, 100 - pane.scrollTop, width, 200);
      if (this.parentElement === list) {
        let x = 10, y = 100 - pane.scrollTop;
        for (const child of list.children) {
          if (x > 10 && x + child.size > width + 10) { x = 10; y += 38; }
          if (child === this) return rect(x, y, child.size, 30);
          x += child.size + 8;
        }
      }
      if (this.parentElement?.classList.contains('detail-tag-chip')) return this.parentElement.getBoundingClientRect();
      return rect(parseFloat(this.style.left) || 10, parseFloat(this.style.top) || 100, parseFloat(this.style.width) || this.size, 30);
    }
    animate(keyframes, options) {
      let complete;
      const finished = new Promise(resolve => { complete = resolve; });
      const animation = {element: this, keyframes, options, finished, complete, cancel: complete};
      animations.push(animation); return animation;
    }
  }
  const shell = new Element(), pane = new Element(), list = new Element();
  shell.dataset.detailGame = 'game'; list.dataset.tagReorder = 'true';
  shell.append(pane); pane.append(list);
  const chips = [60, 110, 80, 170, 70].map((size, i) => {
    const chip = new Element('detail-tag-chip', size), label = new Element('tag-button', size);
    label.dataset.value = `tag-${i}`; label.dataset.action = 'detail-tag';
    chip.append(label); list.append(chip); return chip;
  });
  const side = new Element('tag-edit-button'); side.dataset.action = 'detail-tag-edit'; chips[0].append(side);
  const text = new Element(); chips[0].children[0].append(text);
  list.append(new Element('tag-inline-actions', 40));
  const addEventListener = (name, callback) => {
    if (!listeners.has(name)) listeners.set(name, []);
    listeners.get(name).push(callback);
  };
  const document = {activeElement: null, createElement: () => new Element(), querySelector: () => null, getSelection: () => null, addEventListener};
  const context = vm.createContext({
    document,
    window: {addEventListener}, matchMedia: () => ({matches: reduced}), performance: {now: () => now},
    setTimeout: (callback, delay) => { timers.set(++sequence, {callback, at: now + delay}); return sequence; }, clearTimeout: id => timers.delete(id),
    requestAnimationFrame: callback => { frames.set(++sequence, callback); return sequence; }, cancelAnimationFrame: id => frames.delete(id)
  });
  vm.runInContext(pressSource.replaceAll('export ', ''), context);
  vm.runInContext(source.replace(/^import .*;\r?\n/gm, '').replaceAll('export function', 'function'), context);
  const controller = context.installDetailTagReorder({
    commit(move) { commits.push(JSON.parse(JSON.stringify(move))); return new Promise(resolve => { finishSave = resolve; }); },
    render(html) { renders.push(html); }
  });
  return {
    document, list, pane, chips, text, side, animations, commits, renders, controller,
    get reads() { return reads; },
    get queuedFrames() { return frames.size; },
    get anchor() { return shell.children.find(n => n.classList.contains('tag-drag-layer'))?.children[0]; },
    clickTarget: target => context.detailTagClickTarget({target}),
    save: () => finishSave(),
    order: () => list.querySelectorAll().map(chip => chip.children[0].dataset.value),
    event(name, extra = {}) {
      let prevented = false, stopped = false;
      const event = {target: chips[0].children[0], pointerId: 1, button: 0, isPrimary: true, clientX: 40, clientY: 115,
        preventDefault() { prevented = true; }, stopImmediatePropagation() { stopped = true; }, ...extra};
      for (const callback of listeners.get(name) || []) { callback(event); if (stopped) break; }
      return prevented;
    },
    hold() { const pending = [...timers.values()]; timers.clear(); pending.forEach(({callback, at}) => { now = at; callback(); }); },
    advance(ms) {
      now += ms;
      for (const [id, timer] of timers) if (timer.at <= now) { timers.delete(id); timer.callback(); }
    },
    tick() { now += 16; const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach(callback => callback(now)); },
    async land() { animations.filter(a => a.options.duration === 300).forEach(a => a.complete()); await new Promise(setImmediate); }
  };
}

test('long press reserves a blank slot, shifts variable width rows, then waits for landing and save', async () => {
  const app = fixture();
  app.event('pointerdown'); app.hold();
  assert.ok(app.chips[0].classList.contains('tag-reorder-slot'));
  app.event('pointermove', {clientX: 50, clientY: 150}); app.tick();
  assert.deepEqual(app.order(), ['tag-1', 'tag-2', 'tag-0', 'tag-3', 'tag-4']);
  assert.ok(app.animations.some(a => a.options.duration === 260 && a.keyframes[0].transform !== 'none'));
  assert.equal(app.controller.deferRender('background'), true);
  app.event('pointerup', {clientX: 50, clientY: 150});
  assert.equal(app.commits.length, 0);
  assert.equal(app.event('click'), true);
  assert.ok(app.list.inert);
  await app.land();
  assert.deepEqual(app.commits, [{id: 'game', tag: 'tag-0', before: 'tag-3'}]);
  assert.equal(app.renders.length, 0);
  app.controller.deferRender('saved'); app.save(); await new Promise(setImmediate);
  assert.deepEqual(app.renders, ['saved']);
  assert.equal(app.list.inert, false);
  assert.equal(app.chips[0].classList.contains('tag-reorder-slot'), false);
});

test('short clicks, moving before hold and side buttons do not reorder', () => {
  for (const kind of ['click', 'move', 'side']) {
    const app = fixture();
    app.event('pointerdown', kind === 'side' ? {target: app.side} : {});
    if (kind === 'click') app.event('pointerup');
    if (kind === 'move') app.event('pointermove', {clientX: 55});
    app.hold();
    assert.equal(app.chips[0].classList.contains('tag-reorder-slot'), false);
    assert.equal(app.commits.length, 0);
  }
});

test('text, padding and chip body share click target and the cards’ 380ms hold threshold', () => {
  for (const region of ['label', 'text', 'body']) {
    const app = fixture();
    const label = app.chips[0].children[0];
    const target = region === 'label' ? label : region === 'text' ? app.text : app.chips[0];
    assert.equal(app.clickTarget(target), label);
    app.event('pointerdown', {target});
    app.advance(379);
    assert.equal(app.anchor, undefined);
    app.advance(1);
    assert.ok(app.anchor);
    assert.equal(app.event('click', {target}), true);
  }
});

test('background render does not reset the hold, and side actions stay separate', () => {
  const app = fixture();
  assert.equal(app.clickTarget(app.side), app.side);
  app.event('pointerdown'); app.advance(200);
  assert.equal(app.controller.deferRender('background'), false);
  app.advance(180);
  assert.ok(app.anchor);
});

test('following the pointer coalesces events and needs no geometry reads inside a cached slot', () => {
  const app = fixture();
  app.event('pointerdown'); app.hold();
  assert.equal(app.queuedFrames, 0);
  const reads = app.reads;
  for (let i = 0; i < 100; i++) app.event('pointermove', {clientX: 40 + i % 10});
  assert.equal(app.queuedFrames, 1);
  app.tick();
  assert.equal(app.anchor.style.transform, 'translate3d(19px,92px,0)');
  assert.equal(app.reads, reads);
  assert.equal(app.queuedFrames, 0);
  for (let i = 0; i < 60; i++) app.tick();
  assert.equal(app.reads, reads);
  assert.equal(app.animations.filter(a => a.options.duration === 160).length, 1);
});

test('moving past the press slop cancels both long press and the eventual release click', () => {
  const app = fixture();
  app.event('pointerdown');
  app.event('pointermove', {clientX: 50});
  app.advance(1000);
  app.event('pointerup', {clientX: 50});
  assert.equal(app.anchor, undefined);
  assert.equal(app.event('click'), true);
  assert.equal(app.commits.length, 0);
});

test('pane scroll adjusts cached slots and edge scrolling keeps scheduling until release', async () => {
  const app = fixture({reduced: true});
  app.event('pointerdown'); app.hold();
  const reads = app.reads;
  app.pane.scrollTop = 38;
  app.event('scroll', {target: app.pane}); app.tick();
  assert.ok(app.order().indexOf('tag-0') > 0);
  assert.ok(app.reads > reads); // 新落点触发布局测量，滚动本身不逐帧测量。
  app.event('pointermove', {clientY: 590}); app.tick();
  const scroll = app.pane.scrollTop;
  assert.equal(app.queuedFrames, 1);
  app.tick();
  assert.ok(app.pane.scrollTop > scroll);
  app.event('pointercancel'); await app.land();
  assert.equal(app.queuedFrames, 0);
  assert.equal(app.commits.length, 0);
});

for (const exit of ['Escape', 'pointercancel', 'lostpointercapture', 'blur', 'resize', 'outside']) {
  test(`${exit} animates back without saving`, async () => {
    const app = fixture();
    app.event('pointerdown'); app.hold();
    app.event('pointermove', {clientX: 50, clientY: 150}); app.tick();
    app.event('pointercancel', {pointerId: 9});
    assert.ok(app.chips[0].classList.contains('tag-reorder-slot'));
    if (exit === 'Escape') app.event('keydown', {key: 'Escape'});
    else if (exit === 'outside') app.event('pointerup', {clientX: 700, clientY: 400});
    else app.event(exit);
    assert.ok(app.animations.some(a => a.options.duration === 300));
    await app.land();
    assert.equal(app.commits.length, 0);
    assert.deepEqual(app.order(), ['tag-0', 'tag-1', 'tag-2', 'tag-3', 'tag-4']);
  });
}

test('reduced motion and release before the next frame still commit the final pointer position', async () => {
  const app = fixture({reduced: true});
  app.event('pointerdown'); app.hold();
  app.event('pointerup', {clientX: 290, clientY: 220});
  assert.deepEqual(app.commits, [{id: 'game', tag: 'tag-0', before: null}]);
  assert.equal(app.animations.length, 0);
  app.save(); await new Promise(setImmediate);
});

test('keyboard ordering uses the same intent and respects end boundaries', () => {
  const app = fixture();
  app.chips[0].children[0].focus();
  app.event('keydown', {key: 'ArrowLeft', altKey: true});
  assert.equal(app.commits.length, 0);
  app.event('keydown', {key: 'ArrowRight', altKey: true});
  assert.deepEqual(app.commits, [{id: 'game', tag: 'tag-0', before: 'tag-2'}]);
  assert.equal(app.document.activeElement, app.chips[0].children[0]);
});

for (const cancel of [false, true]) {
  test(`pointer ${cancel ? 'cancellation' : 'drop'} does not leave the tag focused`, async () => {
    const app = fixture();
    app.chips[0].children[0].focus();
    app.event('pointerdown'); app.hold();
    app.event('pointermove', {clientX: 50, clientY: 150}); app.tick();
    if (cancel) app.event('keydown', {key: 'Escape'});
    else app.event('pointerup', {clientX: 50, clientY: 150});
    await app.land();
    if (!cancel) { app.save(); await new Promise(setImmediate); }
    assert.equal(app.document.activeElement, null);
  });
}

test('a stationary pointer keeps a stable insertion slot across wrapped rows', () => {
  for (const width of [220, 300, 600]) {
    for (const [x, y] of [[20, 115], [200, 115], [100, 150], [200, 150], [20, 188], [210, 210]]) {
      const app = fixture({width, reduced: true});
      app.event('pointerdown'); app.hold();
      app.event('pointermove', {clientX: x, clientY: y}); app.tick();
      const order = app.order();
      for (let i = 0; i < 8; i++) app.tick();
      assert.deepEqual(app.order(), order, `width ${width}, pointer ${x},${y}`);
    }
  }
});
