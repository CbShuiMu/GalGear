const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {test} = require('node:test');

const bridge = fs.readFileSync(path.join(__dirname, '../web/bridge.js'), 'utf8');
const clicks = bridge.slice(bridge.indexOf("document.addEventListener('click', event => {"), bridge.indexOf('function setAddMenuExpanded('));
const cardSource = fs.readFileSync(path.join(__dirname, '../web/library-card.js'), 'utf8').replaceAll('export function', 'function');
const tagSource = fs.readFileSync(path.join(__dirname, '../web/detail-tag-reorder.js'), 'utf8');
const tagHitTest = tagSource.slice(tagSource.indexOf('export function detailTagClickTarget'), tagSource.indexOf('// 只管理手势')).replace('export ', '');

function fixture() {
  const calls = [];
  let click;
  const face = {dataset: {action:'game-detail', value:'game'}, closest: () => null};
  const card = {querySelector: () => face};
  const tags = {
    offsetWidth:145, clientWidth:137, offsetHeight:100, clientHeight:100, scrollHeight:200, scrollWidth:137,
    getBoundingClientRect: () => ({right:145,bottom:100}),
    closest: () => card,
  };
  const context = vm.createContext({
    document: {addEventListener: (_, handler) => { click = handler; }, querySelector: () => null},
    performance: {now: () => 100},
    ui: {dispatch: (...args) => calls.push(args)},
  });
  vm.runInContext(`let wasmReady = true, suppressCardClickUntil = 0; ${cardSource} ${tagHitTest} ${clicks}`, context);
  return {
    calls, face, tags,
    suppress() { vm.runInContext('suppressCardClickUntil = 500', context); },
    click(region, coordinates = {}) {
      const action = region === 'title' || region === 'cover' ? face
        : region === 'favorite' ? {dataset: {action:'collection-picker',value:'game'},closest: () => null} : null;
      const tagLabel = {dataset: {action: 'detail-tag', value: '学园'}, closest: () => null};
      const chip = {querySelector: () => tagLabel};
      const target = {closest: selector => {
        if (selector === '.hero-tags .detail-tag-chip:not(.tag-input-chip)' && region.startsWith('tag-')) return chip;
        if (selector === '[data-action]') return region === 'tag-text' ? tagLabel : action;
        return selector === '.card-tags' && region === 'tags' ? tags : null;
      }};
      click({target, clientX:30, clientY:40, preventDefault() {}, stopPropagation() {}, ...coordinates});
    },
  };
}

test('short clicks across title, cover and scrollable tags use the same detail action', () => {
  const app = fixture();
  for (const region of ['title', 'cover', 'tags']) app.click(region);
  assert.deepEqual(app.calls, Array.from({length:3}, () => ['game-detail','game']));
});

test('detail tag text and body padding both filter through the bridge', () => {
  const app = fixture();
  app.click('tag-text'); app.click('tag-padding');
  assert.deepEqual(app.calls, [['detail-tag', '学园'], ['detail-tag', '学园']]);
});

test('bottom actions keep their own commands and scrollbars do not open detail', () => {
  const app = fixture();
  app.click('favorite');
  app.click('tags', {clientX:140});
  app.click('outside');
  assert.deepEqual(app.calls, [['collection-picker','game']]);
});

test('release click after a drag cannot open detail from either shared region', () => {
  const app = fixture(); app.suppress();
  app.click('title'); app.click('cover'); app.click('tags');
  assert.deepEqual(app.calls, []);
});
