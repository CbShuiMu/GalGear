// 在离线页面的浏览器上下文执行；断言实际排版、折叠锚点与引用跳转。
(async () => {
  await document.fonts.ready;
  const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const data = JSON.parse(document.getElementById('route-data').textContent);
  document.querySelector('[data-action="expand"]').click();
  await paint();
  const expandedWidth = document.getElementById('world').offsetWidth;
  const canonical = Array.from(document.querySelectorAll('[data-node]'));
  assert(canonical.length === data.nodes.length, 'Every node must have exactly one full subtree');
  assert(document.querySelectorAll('#links path').length === data.edges.length, 'Every input edge must remain visible when expanded');
  const captions = data.edges.filter(edge => edge.label || edge.condition || edge.effects?.length);
  assert(document.querySelectorAll('.edge-label:not([hidden])').length === captions.length, 'Plain arrows must have no option label');
  // 核对实际箭头落点，同一决策的并行边应进入同一个框的不同位置。
  const targets = Array.from(document.querySelectorAll('.node:not([hidden])'));
  const destinations = new Map();
  for (const wire of document.querySelectorAll('#links path')) {
    const end = wire.getPointAtLength(wire.getTotalLength());
    const target = targets.find(box => {
      const button = box.querySelector('button');
      return (button.dataset.node || button.dataset.jump) === wire.dataset.to &&
        Math.abs(box.offsetTop - end.y) <= 3 &&
        end.x >= box.offsetLeft && end.x <= box.offsetLeft + box.offsetWidth;
    });
    assert(target, 'Arrow missed its target ' + wire.dataset.to);
    const pair = JSON.stringify([wire.dataset.from, wire.dataset.to]);
    if (!destinations.has(pair)) destinations.set(pair, {target, ports: new Set()});
    const group = destinations.get(pair);
    assert(group.target === target && !group.ports.has(end.x), 'Parallel choices need separate arrows into one box');
    group.ports.add(end.x);
  }
  const expectedEffects = data.edges.flatMap(edge => (edge.effects || []).map(({stat,delta}) =>
    JSON.stringify([[edge.label,edge.condition].filter(Boolean).join(' · '), stat + ' ' + (delta > 0 ? '+' : '') + delta])));
  const displayedEffects = Array.from(document.querySelectorAll('.choice-effect'), effect =>
    JSON.stringify([effect.parentElement.firstChild.textContent, effect.textContent]));
  assert(JSON.stringify(expectedEffects.sort()) === JSON.stringify(displayedEffects.sort()), 'A stat change is missing or attached to the wrong option');
  const elements = Array.from(document.querySelectorAll('.node:not([hidden]),.edge-label:not([hidden])'));
  const collisions = [];
  for (let i=0;i<elements.length;i++) for(let j=0;j<i;j++) {
    const a=elements[i].getBoundingClientRect(),b=elements[j].getBoundingClientRect();
    if(a.left<b.right-.1&&a.right>b.left+.1&&a.top<b.bottom-.1&&a.bottom>b.top+.1) collisions.push([elements[i].textContent,elements[j].textContent]);
  }
  assert(!collisions.length, JSON.stringify(collisions));
  const node=canonical.find(b=>b.hasAttribute('aria-expanded'));
  if(node){const before=node.getBoundingClientRect();node.click();await paint();const after=node.getBoundingClientRect();assert(Math.abs(before.x-after.x)<1&&Math.abs(before.y-after.y)<1,'Clicked node moved during collapse');node.click();}
  for(const ref of document.querySelectorAll('[data-jump]')) {
    ref.click();await paint();
    assert(document.activeElement.dataset.node===ref.dataset.jump,'Reference did not focus its canonical target');
    assert(!document.activeElement.closest('.node').hidden,'Reference target remains collapsed');
  }
  assert(!document.querySelector('.node.save'), 'A save slot was rendered as a story node');
  const picker = document.querySelector('.route-picker');
  if (picker) {
    for (const route of data.focus_routes) {
      const entry = canonical.find(node => node.dataset.node === route.entry);
      entry.click();
      await paint();
      assert(picker.value === route.entry, 'Clicking a personal route should activate it');
      assert(entry.getAttribute('aria-pressed') === 'true', 'Active route entry must be indicated');
      assert(canonical.every(node => node.parentElement.classList.contains('route-muted') !== route.nodes.includes(node.dataset.node)), 'Incorrect route node opacity');
      for (const wire of document.querySelectorAll('#links path')) {
        const id = Number(wire.dataset.edge);
        const dim = !route.edges.includes(id);
        assert(wire.classList.contains('route-muted') === dim, 'Incorrect highlighted arrow');
        const label = document.querySelector('.edge-label[data-edge="' + id + '"]');
        if (label) assert(label.classList.contains('route-muted') === dim, 'Incorrect highlighted choice');
      }
      const steps = [...document.querySelectorAll('.route-steps button')];
      assert(steps.length === (route.choices || []).length, 'Guide is missing a selection');
      if (steps.length) {
        steps[0].click();
        assert(document.activeElement.dataset.node === data.edges[route.choices[0]].from, 'Guide step did not locate its choice');
      }
    }
    document.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape'}));
    assert(picker.value === '' && !document.querySelector('.route-muted'), 'Escape should restore the map');
  }
  const localResource = ({name}) => {
    const url = new URL(name);
    return ['file:', 'data:'].includes(url.protocol) ||
      (window.GalGearRoute && url.origin === location.origin);
  };
  assert(performance.getEntriesByType('resource').every(localResource),'Offline document fetched external resources');
  document.querySelector('[data-action="collapse"]').click();
  await paint();
  return {nodes:data.nodes.length,edges:data.edges.length,references:document.querySelectorAll('[data-jump]').length,collisions,validation:data.validation,visibleCollapsed:document.querySelectorAll('.node:not([hidden])').length,width:innerWidth,expandedWidth};
})();
