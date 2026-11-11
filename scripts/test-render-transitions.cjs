const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {test} = require('node:test');

const source = fs.readFileSync('web/bridge.js', 'utf8');
const render = source.slice(source.indexOf('  render(html) {'), source.indexOf('  report_panic(message)'));

function fixture() {
  const applied = [], transitions = [];
  const dataset = {theme: 'light', colorTheme: 'pink'};
  const context = vm.createContext({
    detailTagReorder: {deferRender: () => false}, marqueeDrag: null, dragPayload: '',
    activeThemeTransition: null, renderGeneration: 0, segmentedAppearanceChange: false,
    renderFrame: html => applied.push(html),
    matchMedia: () => ({matches: false}),
    document: {
      documentElement: {dataset},
      querySelector: () => null,
      startViewTransition(update) {
        const transition = {update, ready: Promise.resolve(), finished: Promise.resolve(),
          skipTransition() { this.skipped = true; }};
        transitions.push(transition);
        return transition;
      },
    },
  });
  vm.runInContext(`globalThis.bridge = {${render}}`, context);
  return {bridge: context.bridge, applied, transitions, dataset};
}

test('a delayed theme transition cannot restore an older view after a newer render', () => {
  const {bridge, applied, transitions, dataset} = fixture();
  const first = '<div data-theme="dark" data-color-theme="pink">old view</div>';
  const latest = '<div data-theme="dark" data-color-theme="pink">latest view</div>';
  bridge.render(first);
  bridge.render(latest);
  assert.equal(transitions[0].skipped, true);
  transitions[1].update();
  transitions[0].update();
  assert.deepEqual(applied, [latest]);
  assert.equal(dataset.theme, 'dark');
});

test('an immediate render also invalidates a queued theme change', () => {
  const {bridge, applied, transitions, dataset} = fixture();
  bridge.render('<div data-theme="dark" data-color-theme="pink">old view</div>');
  const latest = '<div data-theme="light" data-color-theme="pink">latest view</div>';
  bridge.render(latest);
  transitions[0].update();
  assert.equal(transitions[0].skipped, true);
  assert.deepEqual(applied, [latest]);
  assert.equal(dataset.theme, 'light');
});
