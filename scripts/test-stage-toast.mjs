import assert from 'node:assert/strict';
import {test} from 'node:test';

let instance = 0;
async function fixture(reduced = false) {
  const media = {matches: reduced, listeners: [], addEventListener(_, fn) { this.listeners.push(fn); }};
  globalThis.window = {matchMedia: () => media};
  const {reconcileStageToast} = await import(`../web/stage-toast.js?test=${++instance}`);
  const animations = [];
  class Text {
    constructor(text, className = 'stage-toast-current') {
      this.textContent = text;
      this.className = className;
      this.attributes = {};
    }
    cloneNode() { return new Text(this.textContent, this.className); }
    setAttribute(key, value) { this.attributes[key] = value; }
    remove() { this.parent.children = this.parent.children.filter(child => child !== this); }
    animate(frames, options) {
      let resolve, reject;
      const animation = {frames, options, cancelled: false,
        finished: new Promise((yes, no) => { resolve = yes; reject = no; }),
        cancel() { this.cancelled = true; reject(new Error('cancelled')); },
        complete() { resolve(); },
      };
      animations.push(animation);
      return animation;
    }
  }
  const node = text => ({
    children: [new Text(text)], dataset: {},
    querySelector(selector) { return this.children.find(child => `.${child.className}` === selector); },
    append(child) { child.parent = this; this.children.push(child); },
  });
  const current = node('Reading');
  return {current, animations, media,
    update(text, immediate = false) {
      const next = node(text);
      next.dataset.immediate = String(immediate);
      reconcileStageToast(current, next);
    },
    reduce() { media.matches = true; media.listeners.forEach(fn => fn()); },
  };
}

test('latest stage interrupts rather than queues and repeated renders preserve its animation', async () => {
  const app = await fixture();
  const textNode = app.current.children[0];
  app.update('Downloading');
  assert.equal(app.animations[0].options.duration, 300);
  assert.equal(app.current.children[1].attributes['aria-hidden'], 'true');
  app.update('Saving');
  assert.ok(app.animations.slice(0, 2).every(animation => animation.cancelled));
  assert.equal(app.current.children.length, 2);
  assert.equal(textNode.textContent, 'Saving');
  assert.equal(app.current.children[1].textContent, 'Downloading');
  app.update('Saving');
  assert.equal(app.animations.length, 4);
  assert.equal(app.current.children[0], textNode);
  app.animations.slice(2).forEach(animation => animation.complete());
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(app.current.children.length, 1);
  assert.equal(textNode.textContent, 'Saving');
});

test('errors interrupt animation immediately and are assigned as text', async () => {
  const app = await fixture();
  app.update('Downloading');
  app.update('Error: <script> & disk full', true);
  assert.equal(app.current.children.length, 1);
  assert.equal(app.current.children[0].textContent, 'Error: <script> & disk full');
  assert.ok(app.animations.every(animation => animation.cancelled));
});

test('reduced motion replaces text and changing the preference finishes an active transition', async () => {
  const reduced = await fixture(true);
  reduced.update('Saving');
  assert.equal(reduced.animations.length, 0);
  const app = await fixture();
  app.update('Extracting');
  app.reduce();
  assert.equal(app.current.children.length, 1);
  assert.equal(app.current.children[0].textContent, 'Extracting');
  app.update('Reading scripts');
  assert.equal(app.animations.length, 2);
});
