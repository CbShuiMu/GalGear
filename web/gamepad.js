import {focusable, focusNode, topLayer} from './navigation.js';

export function direction(axes, buttons) {
  if (buttons[12]?.pressed || axes[1] < -.55) return 'up';
  if (buttons[13]?.pressed || axes[1] > .55) return 'down';
  if (buttons[14]?.pressed || axes[0] < -.55) return 'left';
  if (buttons[15]?.pressed || axes[0] > .55) return 'right';
  return '';
}
export function nearest(origin, candidates, direction) {
  const vertical = direction === 'up' || direction === 'down';
  const sign = direction === 'left' || direction === 'up' ? -1 : 1;
  const center = rect => [rect.left + rect.width / 2, rect.top + rect.height / 2];
  const from = center(origin);
  let best = null, score = Infinity;
  for (const candidate of candidates) {
    const to = center(candidate.rect);
    const primary = (to[vertical ? 1 : 0] - from[vertical ? 1 : 0]) * sign;
    const cross = Math.abs(to[vertical ? 0 : 1] - from[vertical ? 0 : 1]);
    if (primary < 4) continue;
    const rank = primary + cross * 3;
    if (rank < score) { best = candidate.node; score = rank; }
  }
  return best;
}

function move(direction) {
  const nodes = focusable();
  const active = document.activeElement;
  if (!nodes.includes(active)) { focusNode(nodes[0]); return; }
  focusNode(nearest(active.getBoundingClientRect(), nodes.filter(node => node !== active).map(node => ({node, rect:node.getBoundingClientRect()})), direction));
}

let keyboard = null;
function textKeyboard(input) {
  if (keyboard) return;
  const chinese = document.documentElement.lang.startsWith('zh');
  let value = input.value, upper = false;
  const overlay = document.createElement('section');
  overlay.className = 'controller-keyboard';
  const theme = getComputedStyle(document.querySelector('.app-shell'));
  for (const name of ['--surface-raised','--button-color','--button-line','--button-bg','--pink']) overlay.style.setProperty(name,theme.getPropertyValue(name));
  overlay.setAttribute('role','dialog');
  overlay.setAttribute('aria-modal','true');
  overlay.setAttribute('aria-label',chinese ? '手柄键盘' : 'Controller keyboard');
  const preview = document.createElement('output');
  const keys = document.createElement('div'); keys.className = 'controller-keys';
  overlay.append(preview,keys);
  const updatePreview = () => { preview.textContent = input.type === 'password' ? '•'.repeat(value.length) : value || '…'; };
  const finish = commit => {
    overlay.remove(); keyboard = null;
    if (input.isConnected) {
      input.focus({preventScroll:true});
      if (commit) { input.value = value; input.dispatchEvent(new Event('input',{bubbles:true})); input.dispatchEvent(new Event('change',{bubbles:true})); }
    }
  };
  keyboard = {cancel:() => finish(false)};
  function build() {
    keys.replaceChildren(); updatePreview();
    const characters = (upper ? '1234567890QWERTYUIOPASDFGHJKLZXCVBNM' : '1234567890qwertyuiopasdfghjklzxcvbnm') + '.,:/\\-_@!?+=()[]{}#$%&*;\'"<>|~`^';
    const add = (label, action) => { const key=document.createElement('button');key.type='button';key.textContent=label;key.onclick=action;keys.append(key); };
    for (const character of characters) add(character,() => {value += character;updatePreview();});
    add('⇧',() => {upper=!upper;build();focusNode(keys.firstElementChild);});
    add(chinese?'空格':'Space',() => {value+=' ';updatePreview();});
    if (input.tagName === 'TEXTAREA') add('↵',() => {value+='\n';updatePreview();});
    add('⌫',() => {value=Array.from(value).slice(0,-1).join('');updatePreview();});
    add(chinese?'清空':'Clear',() => {value='';updatePreview();});
    add(chinese?'取消':'Cancel',() => finish(false));
    add(chinese?'完成':'Done',() => finish(true));
  }
  build(); document.body.append(overlay); focusNode(keys.firstElementChild);
}

export function startGamepad() {
  let previous = [], lastDirection = '', repeatAt = 0, lastFrame = 0;
  function frame(now) {
    requestAnimationFrame(frame);
    const elapsed = Math.min(40, now - lastFrame); lastFrame = now;
    if (document.hidden) { previous=[];lastDirection='';return; }
    const pad = Array.from(navigator.getGamepads?.() || []).find(pad => pad?.connected && pad.mapping === 'standard');
    if (!pad) {previous=[];lastDirection='';return;}
    const pressed = index => pad.buttons[index]?.pressed && !previous[index];
    const dir = direction(pad.axes, pad.buttons);
    if (dir || pad.buttons.some(button => button.pressed) || Math.abs(pad.axes[3] || 0) > .2) document.body.classList.add('controller-active');
    if (dir && (dir !== lastDirection || now >= repeatAt)) {
      move(dir); repeatAt=now+(dir !== lastDirection ? 360 : 140);
    }
    lastDirection = dir;
    if (pressed(0)) {
      const nodes = focusable();
      const active = document.activeElement;
      if (!nodes.includes(active)) focusNode(nodes[0]);
      else if (active.matches('textarea,input:not([type=checkbox]):not([type=radio]):not([type=range]):not([type=button])')) textKeyboard(active);
      else active.click();
    }
    if (pressed(1)) {
      if (keyboard) keyboard.cancel();
      else {
        const menu = document.activeElement?.closest('details[open]');
        if (menu) {menu.open=false;focusNode(menu.querySelector('summary'));}
        else document.activeElement?.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
      }
    }
    if (pressed(4) || pressed(5)) {
      const nodes=focusable(); const index=nodes.indexOf(document.activeElement);
      focusNode(nodes[(index+(pressed(5)?1:-1)+nodes.length)%nodes.length]);
    }
    const active=document.activeElement;
    if (active?.matches('input[type=range]') && (pressed(6)||pressed(7))) {
      pressed(7) ? active.stepUp() : active.stepDown();active.dispatchEvent(new Event('input',{bubbles:true}));active.dispatchEvent(new Event('change',{bubbles:true}));
    }
    const axis = pad.axes[3] || 0;
    if (Math.abs(axis) > .2) {
      const layer=topLayer();
      const scroll=active?.closest('.editor-body,.library-dialog-body,.detail-scroll,.page-scroll,.setup-table-scroll,.choice-list') || layer?.querySelector('.editor-body,.library-dialog-body,.detail-scroll,.page-scroll,.setup-table-scroll,.choice-list') || layer;
      if (scroll) scroll.scrollTop += axis * elapsed * 1.1;
    }
    previous=pad.buttons.map(button => button.pressed);
  }
  document.addEventListener('keydown',event => { if (keyboard && event.key === 'Escape') { event.stopImmediatePropagation();event.preventDefault();keyboard.cancel(); } }, true);
  document.addEventListener('pointerdown',() => document.body.classList.remove('controller-active'));
  requestAnimationFrame(frame);
}
