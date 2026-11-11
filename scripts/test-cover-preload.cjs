const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {test} = require('node:test');

const source = fs.readFileSync('web/bridge.js', 'utf8');
const listener = source.slice(source.indexOf("    await window.__TAURI__.event.listen('cover-enhanced'"), source.indexOf("    await window.__TAURI__.event.listen('enrichment-progress'"));

async function fixture(selected, fail = false, defer = false) {
  const decoded = [], applied = [], errors = [];
  const pending = [];
  let handler;
  const context = vm.createContext({
    window: {__TAURI__: {event: {listen: (_, callback) => { handler = callback; }}}, GalGearDiagnostics: {error: error => errors.push(error)}},
    document: {querySelector: () => selected === null ? null : {dataset: {value: selected}}},
    Image: class {
      async decode() {
        decoded.push(this.src);
        if (fail) throw new Error('unavailable');
        if (defer) await new Promise(resolve => pending.push(resolve));
      }
    },
    ui: {cover_enhanced: data => applied.push(JSON.parse(data))},
    console: {warn() {}},
  });
  await vm.runInContext(`(async () => {${listener}})()`, context);
  const payload = {id: 'game', previous_cover: 'old', cover: 'cover', cover_artwork: {background: 'background'}};
  const completed = handler({payload});
  if (!defer) await completed;
  return {decoded, applied, errors, payload, pending, completed};
}

test('selected home artwork is fully decoded before its replacement is applied', async () => {
  const app = await fixture('game', false, true);
  assert.deepEqual(app.decoded, ['cover', 'http://galgear-cover.localhost/background']);
  assert.deepEqual(app.applied, []);
  app.pending[0]();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(app.applied, [], 'the cover alone must not replace the selected artwork');
  app.pending[1]();
  await app.completed;
  assert.deepEqual(app.applied, [app.payload]);
});

test('unselected games and other pages preload only the cover', async () => {
  for (const selected of ['other', null]) {
    const app = await fixture(selected);
    assert.deepEqual(app.decoded, ['cover']);
    assert.deepEqual(app.applied, [app.payload]);
  }
});

test('failed preload retains the current artwork and reports the failure', async () => {
  const app = await fixture('game', true);
  assert.deepEqual(app.applied, []);
  assert.equal(app.errors.length, 1);
});
