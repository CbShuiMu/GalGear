const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {test} = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../web/library-reorder.js'), 'utf8');

function fixture({reducedMotion = false} = {}) {
  const listeners = new Map(), frames = new Map(), timers = new Map();
  const animations = [], commits = [];
  let sequence = 0, finishes = 0, resolveSave;
  class Element {
    constructor(className = '') {
      this.className = className;
      this.children = [];
      this.dataset = {};
      this.hidden = false;
      this.inert = false;
      this.scrollTop = 0;
      this.captures = new Set();
      this.style = {setProperty(name, value) { this[name] = value; }};
      this.classList = {
        contains: name => this.className.split(' ').includes(name),
        add: (...names) => { this.className = [...new Set([...this.className.split(' '), ...names])].join(' '); },
        remove: (...names) => { this.className = this.className.split(' ').filter(name => !names.includes(name)).join(' '); },
      };
    }
    get isConnected() { return !!this.parentElement; }
    append(child) { this.insertBefore(child, null); }
    insertBefore(child, before) {
      child.remove();
      const index = before ? this.children.indexOf(before) : this.children.length;
      assert.ok(index >= 0);
      this.children.splice(index, 0, child);
      child.parentElement = this;
    }
    remove() {
      if (this.parentElement) {
        const siblings = this.parentElement.children;
        siblings.splice(siblings.indexOf(this), 1);
        this.parentElement = null;
      }
    }
    setAttribute() {}
    removeAttribute() {}
    setPointerCapture(id) { this.captures.add(id); }
    hasPointerCapture(id) { return this.captures.has(id); }
    releasePointerCapture(id) { this.captures.delete(id); }
    focus() {}
    closest(selector) {
      if (selector === '.game-card') return this.classList.contains('game-card') ? this : null;
      if (selector.startsWith('.game-grid')) return grid;
      if (selector === '.app-shell') return shell;
      return null;
    }
    querySelectorAll(selector) {
      if (selector === '.game-card') return this.children.filter(child => child.classList.contains('game-card'));
      return [];
    }
    querySelector(selector) {
      if (selector === '.selection-check') return this.children.find(child => child.className === 'selection-check');
      if (selector === '.game-face') return this;
      return null;
    }
    cloneNode() {
      const copy = new Element(this.className);
      copy.dataset = {...this.dataset};
      this.children.forEach(child => copy.append(child.cloneNode()));
      return copy;
    }
    getBoundingClientRect() {
      let x = 0, y = 100, width = 200, height = 280;
      if (this === grid || this === pane) { width = 960; height = 650; }
      else if (this.parentElement === grid) {
        const index = grid.children.filter(child => !child.hidden).indexOf(this);
        x = (index % 4) * 240;
        y += Math.floor(index / 4) * 300;
      } else if (this.parentElement?.className === 'reorder-stack') {
        const [left, top] = this.parentElement.style.transform.match(/-?\d+(?:\.\d+)?/g).map(Number);
        x = left + Number(this.style['--stack-index']) * 12;
        y = top;
        width = 150;
        height = 210;
      } else if (this.classList.contains('reorder-flight')) {
        x = parseFloat(this.style.left); y = parseFloat(this.style.top);
        width = parseFloat(this.style.width); height = parseFloat(this.style.height);
      }
      return {x, y, width, height, left: x, top: y, right: x + width, bottom: y + height};
    }
    animate(keyframes, options) {
      let complete;
      const finished = new Promise(resolve => { complete = resolve; });
      const animation = {element: this, keyframes, options, finished, complete, cancel: complete};
      animations.push(animation);
      return animation;
    }
    getAnimations() { return animations.filter(animation => animation.element === this); }
  }
  const shell = new Element('app-shell'), grid = new Element('game-grid'), pane = new Element();
  shell.append(grid);
  const cards = ['a', 'b', 'c', 'd', 'e'].map(id => {
    const card = new Element('game-card is-selected is-selectable');
    card.dataset.gameId = id;
    card.append(new Element('selection-check'));
    grid.append(card);
    return card;
  });
  const document = {
    body: shell,
    createElement: () => new Element(),
    getElementById: () => pane,
    querySelector: () => null,
    addEventListener(name, handler) { listeners.set(name, handler); },
  };
  const context = vm.createContext({
    document, window: {addEventListener: document.addEventListener},
    matchMedia: () => ({matches: reducedMotion}),
    setTimeout(callback) { timers.set(++sequence, callback); return sequence; },
    clearTimeout: id => timers.delete(id),
    requestAnimationFrame(callback) { frames.set(++sequence, callback); return sequence; },
    cancelAnimationFrame: id => frames.delete(id),
  });
  vm.runInContext(source.replace('export function', 'function'), context);
  context.installLibraryReorder({
    idsFor: () => JSON.stringify(['a', 'c']),
    start() {},
    finish() { finishes++; },
    commit(ids, before) {
      commits.push({ids: Array.from(ids), before});
      return new Promise(resolve => { resolveSave = resolve; });
    },
  });
  return {
    cards, grid, shell, animations, commits,
    get finishes() { return finishes; },
    save: () => resolveSave(),
    event(type, extra = {}) {
      listeners.get(type)({target: cards[0], pointerId: 1, isPrimary: true, button: 0, clientX: 100, clientY: 200,
        preventDefault() {}, stopImmediatePropagation() {}, ...extra});
    },
    hold() { const pending = [...timers.values()]; timers.clear(); pending.forEach(callback => callback()); },
  };
}

test('group landing reserves destinations, removes preview selection and waits for animation and persistence', async () => {
  const app = fixture();
  app.event('pointerdown'); app.hold();
  const stack = app.shell.children.find(child => child.className === 'reorder-stack');
  assert.equal(stack.children.length, 2);
  for (const copy of stack.children) {
    assert.equal(copy.classList.contains('is-selected'), false);
    assert.equal(copy.querySelector('.selection-check'), undefined);
  }
  // Release at a new location before requestAnimationFrame processes a move.
  app.event('pointerup', {clientX: 650});
  assert.deepEqual(app.commits, [{ids: ['a', 'c'], before: 'e'}]);
  assert.deepEqual(app.grid.children.map(card => card.dataset.gameId), ['b', 'd', 'a', 'c', 'e']);
  assert.equal(app.grid.children.filter(card => card.classList.contains('reorder-landing')).length, 2);
  const flights = app.animations.filter(animation => animation.options.duration === 420);
  assert.equal(flights.length, 2);
  assert.ok(flights.every(animation => animation.options.easing === 'cubic-bezier(.4,0,.2,1)'));
  assert.notEqual(flights[0].keyframes[1].transform, flights[1].keyframes[1].transform);
  flights.forEach(animation => animation.complete());
  await new Promise(setImmediate);
  assert.equal(app.finishes, 0);
  app.save();
  await new Promise(setImmediate);
  assert.equal(app.finishes, 1);
  assert.equal(app.grid.inert, false);
  assert.ok(app.cards.every(card => !card.hidden && card.classList.contains('is-selected')));
  assert.equal(app.shell.children.length, 1);
});

test('unrelated pointer cancellation cannot discard a collected group, Escape restores it', () => {
  const app = fixture();
  app.event('pointerdown'); app.hold();
  app.event('pointercancel', {pointerId: 2});
  assert.equal(app.cards.filter(card => card.hidden).length, 2);
  app.event('keydown', {key: 'Escape'});
  assert.ok(app.cards.every(card => !card.hidden));
  assert.equal(app.commits.length, 0);
  assert.equal(app.finishes, 1);
});

test('reduced motion keeps destination and persistence behavior without animations', async () => {
  const app = fixture({reducedMotion: true});
  app.event('pointerdown'); app.hold();
  app.event('pointerup', {clientX: 650});
  assert.equal(app.animations.length, 0);
  assert.equal(app.finishes, 0);
  app.save();
  await new Promise(setImmediate);
  assert.equal(app.finishes, 1);
  assert.ok(app.cards.every(card => !card.hidden));
});
