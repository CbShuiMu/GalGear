// 在离线页面的浏览器上下文执行；断言实际排版、折叠锚点与引用跳转。
(async () => {
  await document.fonts.ready;
  const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const data = JSON.parse(document.getElementById('route-data').textContent);
  document.querySelector('[data-action="expand"]').click();
  await paint();
  const canonical = Array.from(document.querySelectorAll('[data-node]'));
  assert(canonical.length === data.nodes.length, 'Every node must have exactly one full subtree');
  assert(document.querySelectorAll('#links path').length === data.edges.length, 'Every input edge must remain visible when expanded');
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
  assert(performance.getEntriesByType('resource').every(r=>r.name.startsWith('file:')||r.name.startsWith('data:')),'Offline document fetched external resources');
  document.querySelector('[data-action="collapse"]').click();
  await paint();
  return {nodes:data.nodes.length,edges:data.edges.length,references:document.querySelectorAll('[data-jump]').length,collisions,validation:data.validation,visibleCollapsed:document.querySelectorAll('.node:not([hidden])').length,width:innerWidth};
})();
