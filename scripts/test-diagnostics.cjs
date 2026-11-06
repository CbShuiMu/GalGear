const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('src-tauri/src/diagnostics/frontend.js', 'utf8');

function setup(handler, loggingFails = false) {
  const records = [], calls = [], listeners = {};
  const window = {
    __TAURI_INTERNALS__: {},
    addEventListener: (name, callback) => { listeners[name] = callback; },
  };
  Object.defineProperty(window.__TAURI_INTERNALS__, 'invoke', {value: async (command, args, options) => {
    if (command === 'record_diagnostic') {
      if (loggingFails) throw new Error('logger unavailable');
      records.push(args.event); return;
    }
    calls.push({command, args, options}); return handler(command, args);
  }});
  vm.runInNewContext(source, {window, performance, Headers, navigator: {userAgent: 'test'}, Error});
  return {api: window.GalGearDiagnostics, records, calls, listeners};
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
