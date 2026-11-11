const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {test} = require('node:test');

const source = fs.readFileSync('web/bridge.js', 'utf8');
const reconcileChildren = source.slice(source.indexOf('function reconcileChildren('), source.indexOf('function alignLibraryToolbar('));

class Element {
  constructor(key, value = key, tag = 'button') {
    Object.assign(this, {key, value, tag, childNodes: [], parent: null});
  }
  get firstChild() { return this.childNodes[0] || null; }
  get nextSibling() { return this.parent?.childNodes[this.parent.childNodes.indexOf(this) + 1] || null; }
  insertBefore(node, cursor) {
    node.remove();
    const index = cursor ? this.childNodes.indexOf(cursor) : this.childNodes.length;
    assert.ok(index >= 0);
    this.childNodes.splice(index, 0, node);
    node.parent = this;
  }
  remove() {
    if (this.parent) this.parent.childNodes.splice(this.parent.childNodes.indexOf(this), 1);
    this.parent = null;
  }
  cloneNode() { return new Element(this.key, this.value, this.tag); }
}

function parent(children) {
  const node = new Element(null);
  children.forEach(child => node.insertBefore(child, null));
  return node;
}

function fixture() {
  let comparisons = 0;
  const context = vm.createContext({
    renderKey: node => node.key,
    compatible: (a, b) => { comparisons++; return a.key === b.key && a.tag === b.tag; },
    reconcile: (a, b) => { a.value = b.value; },
  });
  vm.runInContext(reconcileChildren, context);
  return {render: context.reconcileChildren, comparisons: () => comparisons};
}

test('large reversals retain every node and use a linear number of matching comparisons', () => {
  const nodes = Array.from({length: 1200}, (_, i) => new Element(String(i)));
  const current = parent(nodes);
  const next = parent(nodes.toReversed().map(node => new Element(node.key, `updated-${node.key}`)));
  const app = fixture();
  app.render(current, next);
  assert.deepEqual(current.childNodes, nodes.toReversed());
  assert.equal(current.firstChild.value, 'updated-1199');
  assert.ok(app.comparisons() < nodes.length * 3, app.comparisons());
});

test('duplicate action keys consume each old control once and clone additional controls', () => {
  const first = new Element('edit', 'first');
  const second = new Element('edit', 'second');
  const current = parent([new Element('obsolete'), first, second]);
  const next = parent([new Element('edit', 'a'), new Element('edit', 'b'), new Element('edit', 'c')]);
  fixture().render(current, next);
  assert.equal(current.childNodes.length, 3);
  assert.equal(current.childNodes[0], first);
  assert.equal(current.childNodes[1], second);
  assert.notEqual(current.childNodes[2], first);
  assert.deepEqual(current.childNodes.map(node => node.value), ['a', 'b', 'c']);
});

test('mixed node types and unkeyed content retain matching controls and remove obsolete nodes', () => {
  const input = new Element('entry', 'draft', 'input');
  const current = parent([new Element(null, 'old text', '#text'), new Element('entry'), input]);
  const next = parent([new Element('entry', 'saved', 'input'), new Element(null, 'new text', '#text')]);
  fixture().render(current, next);
  assert.equal(current.childNodes.length, 2);
  assert.equal(current.firstChild, input);
  assert.deepEqual(current.childNodes.map(node => node.value), ['saved', 'new text']);
  fixture().render(current, parent([]));
  assert.equal(current.childNodes.length, 0);
});
