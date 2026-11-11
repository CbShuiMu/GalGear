const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('src-tauri/src/diagnostics/frontend.js', 'utf8');

function setup(handler, loggingFails = false) {
  const records = [], calls = [], listeners = {};
  const frames = new Map(), timers = new Map();
  let nextCallback = 0;
  const schedule = queue => callback => { queue.set(++nextCallback, callback); return nextCallback; };
  const tick = queue => {
    const callbacks = [...queue.values()];
    queue.clear();
    callbacks.forEach(callback => callback());
  };
  const document = {
    visibilityState: 'visible',
    addEventListener: (name, callback) => { listeners[name] = callback; },
  };
  const window = {
    __TAURI_INTERNALS__: {},
    addEventListener: (name, callback) => { listeners[name] = callback; },
    requestAnimationFrame: schedule(frames),
    cancelAnimationFrame: id => frames.delete(id),
    setTimeout: schedule(timers),
    clearTimeout: id => timers.delete(id),
  };
  Object.defineProperty(window.__TAURI_INTERNALS__, 'invoke', {value: async (command, args, options) => {
    if (command === 'record_diagnostic') {
      if (loggingFails) throw new Error('logger unavailable');
      records.push(args.event); return;
    }
    calls.push({command, args, options}); return handler(command, args);
  }});
  vm.runInNewContext(source, {window, document, performance, Headers, navigator: {userAgent: 'test'}, Error});
  return {api: window.GalGearDiagnostics, records, calls, listeners, document,
    advanceFrame: () => tick(frames), expireTimers: () => tick(timers)};
}

test('correlates concurrent completions without recording payloads', async () => {
  const {api, records, calls} = setup(async command => ({token: `${command}-secret`}));
  const results = await Promise.all([api.invoke('first', {password: 'private'}), api.invoke('second')]);
  assert.equal(results[0].token, 'first-secret');
  assert.equal(new Set(records.map(record => record.request)).size, 2);
  assert.equal(records[0].request, calls[0].options.headers.get('x-galgear-request'));
  assert.ok(records.every(record => record.kind === 'command_ok'));
  assert.ok(!JSON.stringify(records).includes('secret'));
  assert.ok(!JSON.stringify(records).includes('private'));
});

test('retains original rejection and records structured error', async () => {
  const error = {key: 'error.launch', args: {detail: 'os error 5'}};
  const {api, records} = setup(() => { throw error; });
  await assert.rejects(api.invoke('launch_game'), value => value === error);
  assert.equal(records[0].kind, 'command_error');
  assert.ok(records[0].detail.includes('os error 5'));
});

test('logging failure does not reject successful work or recurse', async () => {
  const {api, calls} = setup(() => 42, true);
  assert.equal(await api.invoke('test'), 42);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls.length, 1);
});

test('captures startup, resource, unhandled rejection and WASM panic diagnostics', async () => {
  const {api, records, listeners} = setup(() => {});
  listeners.error({filename: 'bridge.js', lineno: 42, message: 'initialization failed'});
  listeners.error({target: {src: 'pkg/galgear_ui.js'}});
  listeners.unhandledrejection({reason: new Error('async failed')});
  await api.panic('panicked at view.rs:20');
  assert.ok(records[0].detail.includes('bridge.js:42'));
  assert.ok(records[1].detail.includes('pkg/galgear_ui.js'));
  assert.ok(records[2].detail.includes('async failed'));
  assert.equal(records[3].kind, 'wasm_panic');
  assert.ok(records[3].detail.includes('Error'));
});

test('reports setup dismissal and correlates only the current view with frame callbacks', () => {
  const {api, records, advanceFrame, expireTimers} = setup(() => {});
  api.viewApplied('home', 'setup');
  advanceFrame();
  api.viewApplied('home', 'none');
  api.viewApplied('home', 'none');
  advanceFrame();
  assert.equal(records.length, 2);
  advanceFrame();
  expireTimers();
  assert.deepEqual(records.map(({view}) => [view.sequence, view.page, view.overlay, view.phase]), [
    [1, 'home', 'setup', 'applied'],
    [2, 'home', 'none', 'applied'],
    [2, 'home', 'none', 'frames'],
  ]);
});

test('reports visible frame delay and recovery without recording settings sections', () => {
  const {api, records, advanceFrame, expireTimers} = setup(() => {});
  api.viewApplied('settings:private-section', 'none');
  expireTimers();
  advanceFrame();
  advanceFrame();
  assert.deepEqual(records.map(({view}) => [view.sequence, view.page, view.phase]), [
    [1, 'settings', 'applied'],
    [1, 'settings', 'frames_delayed'],
    [1, 'settings', 'frames'],
  ]);
  assert.ok(!JSON.stringify(records).includes('private-section'));
});

test('cancels pending frame checks when the document becomes hidden', () => {
  const {api, records, listeners, document, advanceFrame, expireTimers} = setup(() => {});
  api.viewApplied('home', 'none');
  document.visibilityState = 'hidden';
  listeners.visibilitychange();
  api.viewApplied('games', 'none');
  expireTimers();
  advanceFrame();
  advanceFrame();
  assert.deepEqual(records.map(({kind, visible, view}) => [kind, visible, view?.phase]), [
    ['view', true, 'applied'],
    ['visibility', false, undefined],
    ['view', false, 'applied'],
  ]);
});
