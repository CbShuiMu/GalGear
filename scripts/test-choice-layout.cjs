const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {test} = require('node:test');
const bridge = fs.readFileSync(require('node:path').join(__dirname, '../web/bridge.js'), 'utf8');
const place = bridge.slice(bridge.indexOf('function placeChoice()'), bridge.indexOf('let detailReturnAnimation'));

function fixture(kind = '') {
  let natural = 140;
  const anchor = {width: 180, left: 20, top: 80, bottom: 118};
  const style = {};
  const width = () => Math.min(Number.parseFloat(style.maxWidth) || Infinity,
    Math.max(Number.parseFloat(style.minWidth) || 0, style.width === 'max-content' ? natural : Number.parseFloat(style.width) || 180));
  // flex 文本槽至少占满当前菜单，scrollWidth 并不等于文字的固有宽度。
  const label = {get scrollWidth() { return Math.max(natural - 60, width() - 60); }};
  const menu = {style, dataset: {choiceAnchor: 'source'}, classList: {contains: value => value === kind},
    querySelectorAll: () => [label], getBoundingClientRect: () => ({width: width(), height: 120})};
  const context = vm.createContext({innerWidth: 1000, innerHeight: 700,
    document: {querySelector: () => menu, getElementById: () => ({getBoundingClientRect: () => anchor})}});
  vm.runInContext(place, context);
  return {style, context, anchor, width, resizeText: value => { natural = value; }, place: () => vm.runInContext('placeChoice()', context)};
}

test('repeated renders do not grow data source dropdowns', () => {
  const menu = fixture();
  menu.place();
  const first = menu.width();
  for (let i = 0; i < 100; i++) menu.place();
  assert.equal(menu.width(), first);
  assert.equal(menu.width(), 180);
});

test('long labels, font changes and narrow windows recompute intrinsic bounds', () => {
  const menu = fixture('setup-menu');
  menu.resizeText(900); menu.place();
  assert.equal(menu.width(), 560);
  menu.resizeText(250); menu.place();
  assert.equal(menu.width(), 250);
  menu.context.innerWidth = 160; menu.anchor.left = 100; menu.place();
  assert.equal(menu.width(), 144);
  assert.equal(menu.style.left, '8px');
});
