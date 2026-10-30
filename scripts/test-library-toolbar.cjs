const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {test} = require('node:test');

const bridge = fs.readFileSync(path.join(__dirname, '../web/bridge.js'), 'utf8');
const align = bridge.slice(bridge.indexOf('function alignLibraryToolbar()'), bridge.indexOf('function measureLibraryBounds()'));

function fixture() {
  let gridWidth = 1800, cardWidth = 312, count = 20, collection = false;
  const card = {getBoundingClientRect: () => ({width: cardWidth, right: 412})};
  const bar = {
    style: {width: '', removeProperty(key) { delete this[key]; }},
    classList: {contains: () => collection},
    getBoundingClientRect: () => ({left: 100}),
  };
  const grid = {
    querySelector: () => count ? card : null,
    getBoundingClientRect: () => ({left: 100, width: gridWidth}),
  };
  const page = {querySelector: selector => selector === '.library-bar' ? bar : grid};
  const context = vm.createContext({
    document: {querySelector: () => page},
    getComputedStyle: () => ({columnGap: '20px'}),
    libraryGridObserver: {observe() {}, disconnect() {}},
  });
  vm.runInContext(`let observedLibraryGrid; ${align}`, context);
  return {
    bar,
    align: () => vm.runInContext('alignLibraryToolbar()', context),
    set(options) {
      gridWidth = options.gridWidth ?? gridWidth;
      cardWidth = options.cardWidth ?? cardWidth;
      count = options.count ?? count;
      collection = options.collection ?? collection;
    },
  };
}

test('full-screen toolbar keeps every grid column when a tag leaves only one result', () => {
  const app = fixture();
  app.align();
  assert.equal(app.bar.style.width, '1640px');
  app.set({count: 1});
  app.align();
  assert.equal(app.bar.style.width, '1640px');
});

test('filtered toolbar follows both window enlargement and shrinking', () => {
  const app = fixture();
  app.set({count: 1, gridWidth: 1000, cardWidth: 227});
  app.align();
  assert.equal(app.bar.style.width, '968px');
  app.set({gridWidth: 1800, cardWidth: 312});
  app.align();
  assert.equal(app.bar.style.width, '1640px');
  app.set({gridWidth: 1000, cardWidth: 227});
  app.align();
  assert.equal(app.bar.style.width, '968px');
});

test('empty results and collection toolbars clear stale inline widths', () => {
  const app = fixture();
  app.align();
  app.set({count: 0});
  app.align();
  assert.equal(app.bar.style.width, undefined);
  app.set({count: 1});
  app.align();
  app.set({collection: true});
  app.align();
  assert.equal(app.bar.style.width, undefined);
});
