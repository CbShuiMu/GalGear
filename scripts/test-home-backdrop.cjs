const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {test} = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../web/home-backdrop.js'), 'utf8');
const flush = () => new Promise(resolve => setImmediate(resolve));

function fixture() {
  const loads = new Map(), animations = [];
  let onMotionChange;
  const motion = {matches: false, addEventListener(_, callback) { onMotionChange = callback; }};
  class Element {
    children = [];
    dataset = {};
    clientWidth = 1000;
    isConnected = true;
    append(node) { node.parent = this; this.children.push(node); }
    remove() {
      if (this.parent) this.parent.children = this.parent.children.filter(node => node !== this);
      this.parent = null;
    }
    animate(frames) {
      let resolve;
      const animation = {node: this, frames, finished: new Promise(done => { resolve = done; }),
        finish() { resolve(); }, cancel() { this.cancelled = true; resolve(); }};
      animations.push(animation);
      return animation;
    }
  }
  class Image extends Element {
    naturalWidth = 600;
    naturalHeight = 900;
    decode() { return new Promise((resolve, reject) => loads.set(this.src, {resolve, reject, image: this})); }
    cloneNode() { const copy = new Image(); copy.src = this.src; return copy; }
  }
  const host = new Element();
  const context = vm.createContext({Image, matchMedia: () => motion,
    document: {createElement: () => new Element()}});
  vm.runInContext(source.replace('export function', 'function'), context);
  const sync = (target, direction = 0) => context.syncHomeBackdrop(target, direction);
  return {
    host, animations, sync,
    select(src, direction = 1) { host.dataset.cover = src; sync(host, direction); },
    async load(src, {fail = false, landscape = false} = {}) {
      const load = loads.get(src);
      assert.ok(load, `Expected pending decode: ${src}`);
      if (landscape) load.image.naturalWidth = 1600;
      if (fail) load.reject(new Error('Unavailable fixture'));
      else load.resolve();
      await flush();
    },
    async finish() { animations.slice().forEach(animation => animation.finish()); await flush(); },
    reduce() { motion.matches = true; onMotionChange(); },
    sources() { return host.children.map(layer => layer.children.at(-1).src); },
  };
}

async function initial() {
  const app = fixture();
  app.select('a', 0);
  await app.load('a');
  await app.finish();
  return app;
}

test('decoded layers fade in opposite horizontal directions and preserve each layout', async () => {
  const app = await initial();
  app.select('b', 1);
  assert.deepEqual(app.sources(), ['a']);
  await app.load('b', {landscape: true});
  assert.deepEqual(app.sources(), ['a', 'b']);
  assert.equal(app.host.children[0].dataset.layout, 'portrait');
  assert.equal(app.host.children[1].dataset.layout, 'landscape');
  assert.equal(app.animations.at(-2).frames[1].transform, 'translateX(-45px)');
  assert.equal(app.animations.at(-1).frames[0].transform, 'translateX(45px)');
  await app.finish();
  assert.deepEqual(app.sources(), ['b']);
  app.select('a', -1);
  await app.load('a');
  assert.equal(app.animations.at(-2).frames[1].transform, 'translateX(45px)');
  assert.equal(app.animations.at(-1).frames[0].transform, 'translateX(-45px)');
});

test('out-of-order decoding cannot replace the final selection', async () => {
  const app = await initial();
  app.select('slow');
  app.select('last');
  await app.load('last');
  await app.finish();
  await app.load('slow');
  assert.deepEqual(app.sources(), ['last']);
});

test('rapid selections during a fade skip intermediate games and converge on the latest', async () => {
  const app = await initial();
  app.select('b');
  await app.load('b');
  app.select('c');
  await app.load('c');
  app.select('d');
  await app.load('d');
  assert.deepEqual(app.sources(), ['a', 'b']);
  await app.finish();
  assert.deepEqual(app.sources(), ['b', 'd']);
  await app.finish();
  assert.deepEqual(app.sources(), ['d']);
});

test('returning to the visible cover invalidates pending loads without restarting a fade', async () => {
  const app = await initial();
  app.select('slow');
  app.select('a', -1);
  await app.load('slow');
  assert.deepEqual(app.sources(), ['a']);
  assert.equal(app.animations.length, 1);
});

test('missing or failed final cover clears the previous game artwork', async () => {
  for (const target of ['', 'broken']) {
    const app = await initial();
    app.select(target);
    if (target) await app.load(target, {fail: true});
    await app.finish();
    assert.deepEqual(app.sources(), []);
  }
});

test('leaving home disposes animations and ignores a late decode even after returning', async () => {
  const app = await initial();
  app.select('b');
  await app.load('b');
  app.select('slow');
  app.sync(null);
  assert.deepEqual(app.sources(), []);
  assert.ok(app.animations.every(animation => animation.cancelled));
  app.select('last');
  await app.load('last');
  await app.load('slow');
  await app.finish();
  assert.deepEqual(app.sources(), ['last']);
});

test('reduced motion settles an active fade and shows the latest decoded cover immediately', async () => {
  const app = await initial();
  app.select('b');
  await app.load('b');
  app.select('c');
  await app.load('c');
  app.reduce();
  assert.deepEqual(app.sources(), ['c']);
  const count = app.animations.length;
  app.select('d');
  await app.load('d');
  assert.deepEqual(app.sources(), ['d']);
  assert.equal(app.animations.length, count);
});
