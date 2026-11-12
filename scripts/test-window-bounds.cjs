const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {test} = require('node:test');

const bridge = fs.readFileSync(path.join(__dirname, '../web/bridge.js'), 'utf8');
const source = bridge.slice(bridge.indexOf('function measureLibraryBounds()'), bridge.indexOf('function placeChoice()'));
const flush = () => new Promise(resolve => setImmediate(resolve));

function fixture() {
  let visible = true, sidebar = 80;
  const calls = [], errors = [];
  const shell = {dataset: {}, getBoundingClientRect: () => ({left: 0})};
  const pane = {offsetWidth: 1000, clientWidth: 986, getBoundingClientRect: () => ({left: sidebar})};
  const page = {querySelector: () => ({})};
  const context = vm.createContext({
    document: {
      querySelector: selector => selector === '.app-shell' ? shell : visible ? page : null,
      getElementById: () => pane,
    },
    getComputedStyle: () => ({getPropertyValue: key => ({
      '--library-card-min-width': '227', '--library-column-gap': '20', '--library-min-content-inset': '30',
    })[key]}),
    alignLibraryToolbar() {},
    window: {__TAURI__: {core: {invoke: (command, bounds) => new Promise((resolve, reject) => {
      calls.push({command, bounds: {...bounds}, resolve, reject});
    })}}},
    console: {error: (...args) => errors.push(args)},
  });
  vm.runInContext(`let minimumRequest = '', minimumBounds = null, minimumPending = false, minimumRetry = false; ${source}`, context);
  return {
    calls, errors, shell,
    measure: () => vm.runInContext('measureLibraryBounds()', context),
    hide() { visible = false; },
    sidebar(value) { sidebar = value; },
  };
}

test('deferred bounds retry after restore, including after leaving the library', async () => {
  const app = fixture();
  app.measure();
  assert.equal(app.calls[0].command, 'set_library_bounds');
  assert.deepEqual(app.calls[0].bounds, {width: 1122, height: 640});
  app.calls[0].resolve(false);
  await flush();
  assert.equal(app.calls.length, 1, 'no polling while maximized');
  app.hide();
  app.measure(); // window resize after restore, on another page
  assert.equal(app.calls.length, 2);
  assert.deepEqual(app.calls[1].bounds, {width: 1122, height: 640});
  app.calls[1].resolve(true);
  await flush();
  app.measure();
  assert.equal(app.calls.length, 2, 'only applied bounds are cached');
});

test('restoring while a deferred response is in flight still retries', async () => {
  const app = fixture();
  app.measure();
  app.measure();
  app.measure();
  assert.equal(app.calls.length, 1);
  app.calls[0].resolve(false);
  await flush();
  assert.equal(app.calls.length, 2);
  app.calls[1].resolve(true);
  await flush();
  assert.equal(app.calls.length, 2);
});

test('collection and layout changes coalesce to the latest bounds', async () => {
  const app = fixture();
  app.measure();
  app.sidebar(300);
  app.measure();
  app.sidebar(320);
  app.measure();
  app.calls[0].resolve(true);
  await flush();
  assert.equal(app.calls.length, 2);
  assert.deepEqual(app.calls[1].bounds, {width: 1362, height: 640});
  app.calls[1].resolve(true);
  await flush();
  app.measure();
  assert.equal(app.calls.length, 2);
});

test('returning to previously applied bounds during a request does not lose the update', async () => {
  const app = fixture();
  app.measure(); app.calls[0].resolve(true); await flush();
  app.sidebar(320); app.measure();
  app.sidebar(80); app.measure();
  app.calls[1].resolve(true); await flush();
  assert.equal(app.calls.length, 3);
  assert.deepEqual(app.calls[2].bounds, {width: 1122, height: 640});
  app.calls[2].resolve(true); await flush();
});

test('failed bounds remain retryable and older null responses remain compatible', async () => {
  const app = fixture();
  app.measure();
  app.calls[0].reject(new Error('native failure'));
  await flush();
  assert.equal(app.errors.length, 1);
  app.measure();
  assert.equal(app.calls.length, 2);
  app.calls[1].resolve(null);
  await flush();
  app.measure();
  assert.equal(app.calls.length, 2);
});

test('a partially failed native update invalidates previously applied bounds', async () => {
  const app = fixture();
  app.measure(); app.calls[0].resolve(true); await flush();
  app.sidebar(320); app.measure();
  app.calls[1].reject(new Error('resize failed after setting minimum'));
  await flush();
  app.sidebar(80); app.measure();
  assert.equal(app.calls.length, 3);
  assert.deepEqual(app.calls[2].bounds, {width: 1122, height: 640});
  app.calls[2].resolve(true); await flush();
});
