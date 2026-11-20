/* 同一决策的并行边指向一个剧情框。其他分支的已有子树及回跳保留定位引用。 */
(() => {
  'use strict';
  const source = document.getElementById('route-data');
  if (!source) return;
  const data = JSON.parse(source.textContent);
  const t = data.ui;
  const board = document.getElementById('board'), world = document.getElementById('world');
  const layer = document.getElementById('nodes'), svg = document.getElementById('wires');
  const links = document.getElementById('links'), detail = document.getElementById('detail');
  const nodes = new Map(data.nodes.map(n => [n.id, n]));
  const outgoing = new Map(data.nodes.map(n => [n.id, data.edges.filter(e => e.from === n.id)]));
  const entries = new Map(), open = new Set(), roots = [];
  const routes = (data.focus_routes || []).map(r => ({...r, nodes: new Set(r.nodes), edges: new Set(r.edges)}));
  const owners = new Map(routes.flatMap(r => r.members.map(id => [id, r])));
  const edgeIndex = new Map(data.edges.map((edge, i) => [edge, i]));
  let activeRoute, picker, guidePanel;
  let optionId = 0;
  const camera = {x:0,y:16,scale:1};
  const branchGap = 28, canvasPadding = 32;
  let dimensions, drag, selected;
  const el = (tag, cls, text) => {const e=document.createElement(tag);e.className=cls||'';if(text)e.textContent=text;return e;};
  document.getElementById('title').textContent = data.title;
  document.getElementById('hint').textContent = t.hint;
  board.setAttribute('aria-label',t.canvas);
  document.getElementById('legend').textContent = t.legend;
  document.getElementById('review-title').textContent = t.review;
  const review=document.getElementById('review');
  for(const s of [...data.validation,...data.uncertainties]) review.append(el('p','',s));
  if(data.validation.length) document.getElementById('review-title').classList.add('warning');
  function effectText({stat, delta}) { return `${stat} ${delta > 0 ? '+' : ''}${delta}`; }
  function edgeText(edge) {
    return [edge.label, edge.condition, ...(edge.effects || []).map(effectText)].filter(Boolean).join(' · ');
  }
  function showDetail(b){
    selected=b;
    detail.textContent=[b.node.title,
      ...b.options.flatMap(({edge}) => [edgeText(edge),edge.pc,edge.evidence,
        ...(edge.scenes || []).flatMap(scene =>
          [scene.title, scene.pc, scene.evidence, scene.transition_pc, scene.transition_evidence])]),
      b.node.pc,b.node.evidence].filter(Boolean).join('\n');
  }
  function create(id, incoming=[], parent=null){
    const node=nodes.get(id), reference=entries.has(id);
    const b={id,node,parent,reference,children:[],options:[]};
    b.element=el('div',`node ${node.type} ${reference?'reference':''}`);
    b.control=el('button','');b.control.type='button';b.control.dataset[reference?'jump':'node']=id;
    b.icon=el('span');b.icon.setAttribute('aria-hidden','true');
    b.control.append(b.icon,document.createTextNode(' '+(node.type==='extra'?'EXTRA · ':node.type==='grand'?'GRAND · ':'')+node.title));
    b.element.append(b.control);layer.append(b.element);
    for (const edge of incoming) {
      const option = {edge};
      if (edgeText(edge)) {
        option.label = el('span', `edge-label ${edge.kind==='unlock'?'unlock':''}`, [edge.label,edge.condition].filter(Boolean).join(' · '));
        for (const effect of edge.effects || []) option.label.append(el('span','choice-effect',effectText(effect)));
        option.label.id = 'option-' + ++optionId;
        option.label.dataset.edge = edgeIndex.get(edge);
        layer.append(option.label);
      }
      b.options.push(option);
    }
    const descriptions = b.options.flatMap(o => o.label ? [o.label.id] : []);
    if (descriptions.length) b.control.setAttribute('aria-describedby', descriptions.join(' '));
    if (!reference) {
      entries.set(id,b);
      const destinations = new Map();
      for (const edge of outgoing.get(id)) {
        if (!destinations.has(edge.to)) destinations.set(edge.to, []);
        destinations.get(edge.to).push(edge);
      }
      for (const [target, edges] of destinations) b.children.push(create(target, edges, b));
    }
    b.control.addEventListener('click',()=>activate(b));
    b.control.addEventListener('focus',()=>reveal(b));
    return b;
  }
  // Start roots first. Any disconnected evidence stays a separate tree, with an explicit validation warning.
  for(const node of data.nodes.filter(n=>n.type==='start'))if(!entries.has(node.id))roots.push(create(node.id));
  for(const node of data.nodes)if(!entries.has(node.id))roots.push(create(node.id));
  function initial(){open.clear();const depth=board.clientWidth<540?2:4;for(const b of entries.values()){let d=0;for(let p=b.parent;p;p=p.parent)d++;if(d<depth)open.add(b.id);}}
  function measure(b, visible){
    b.element.hidden=!visible;
    const included = activeRoute?.nodes.has(b.id) && (!b.reference || b.options.some(({edge}) => activeRoute.edges.has(edgeIndex.get(edge))));
    b.element.classList.toggle('route-muted', !!activeRoute && !included);
    if (owners.has(b.id)) b.control.setAttribute('aria-pressed', String(owners.get(b.id) === activeRoute));
    for (const {edge, label} of b.options) if (label) {
      label.hidden = !visible;
      label.classList.toggle('route-muted', !!activeRoute && !activeRoute.edges.has(edgeIndex.get(edge)));
      label.classList.toggle('route-choice', !!activeRoute && activeRoute.edges.has(edgeIndex.get(edge)));
    }
    const expanded=visible&&open.has(b.id)&&!b.reference;
    b.expanded=expanded;
    b.icon.textContent=b.reference?'↗':b.children.length?(expanded?'▾':'▸'):b.node.type==='start'?'▷':'⚑';
    if(b.children.length)b.control.setAttribute('aria-expanded',String(expanded));
    for(const c of b.children)measure(c,expanded);
    if(!visible)return;
    b.h=b.element.offsetHeight;
    b.w=b.element.offsetWidth;
    let width = 0;
    b.labelHeight = 0;
    for (const option of b.options) {
      option.w = option.label?.offsetWidth || 0;
      option.h = option.label?.offsetHeight || 0;
      option.x = width + option.w / 2;
      width += option.w + branchGap;
      b.labelHeight = Math.max(b.labelHeight, option.h);
    }
    b.labelWidth = Math.max(0, width - branchGap);
    for (const option of b.options) option.x -= b.labelWidth / 2;
  }
  function arrange(b, top) {
    b.top = top;
    const bottom = top + b.h;
    const node = {left: -b.w / 2, right: b.w / 2, top, bottom};
    if (!b.expanded || !b.children.length) return [node];

    const labelTop = b.childLabelTop = bottom + 28;
    b.childLabelHeight = Math.max(...b.children.map(c => c.labelHeight));
    const childTop = labelTop + b.childLabelHeight + 28;
    const placed = [];
    let offset = 0;
    for (const c of b.children) {
      const regions = arrange(c, childTop);
      regions.push({left: -c.labelWidth / 2, right: c.labelWidth / 2, top: labelTop, bottom: childTop});
      // 只在纵向相交的区域比较横向间距，短分支下方可留给相邻分支的后续节点。
      for (const region of regions) {
        for (const other of placed) {
          if (region.top < other.bottom && other.top < region.bottom) {
            offset = Math.max(offset, other.right - region.left + branchGap);
          }
        }
      }
      c.offset = offset;
      for (const region of regions) {
        region.left += offset;
        region.right += offset;
        placed.push(region);
      }
    }

    const center = offset / 2;
    for (const c of b.children) c.offset -= center;
    for (const region of placed) {
      region.left -= center;
      region.right -= center;
    }
    // 并排选项可能宽于共同剧情框，为分支横杆和竖线预留空间。
    return [node, {
      left: Math.min(0, ...b.children.map(c => c.offset - c.labelWidth / 2)),
      right: Math.max(0, ...b.children.map(c => c.offset + c.labelWidth / 2)),
      top: bottom, bottom: labelTop
    }, ...placed];
  }

  function verticalLink(fromX, toX, turnY, endY) {
    return (fromX === toX ? '' : ` V${turnY} H${toX}`) + ` V${endY}`;
  }

  function place(b, x, rootTop) {
    b.x = x;
    b.y = rootTop + b.top;
    b.element.style.left = (x - b.w / 2) + 'px';
    b.element.style.top = b.y + 'px';
    if (!b.expanded || !b.children.length) return;
    const labelY = rootTop + b.childLabelTop;
    const rowBottom = labelY + b.childLabelHeight;
    for (const c of b.children) {
      place(c, x + c.offset, rootTop);
      c.options.forEach((option, index) => {
        const labelX = c.x + option.x;
        if (option.label) {
          option.label.style.left = (labelX - option.w / 2) + 'px';
          option.label.style.top = labelY + 'px';
        }
        const sy = b.y + b.h, ey = c.y - 2;
        const port = c.x + c.w * ((index + 1) / (c.options.length + 1) - 0.5);
        const path = document.createElementNS('http://www.w3.org/2000/svg','path');
        let line = `M${b.x} ${sy}`;
        // 混合标签行中的无标签分支亦保持竖直，横向转折只发生在标签行外。
        if (b.childLabelHeight) {
          line += verticalLink(b.x, labelX, (sy + labelY) / 2, labelY);
          if (option.label) line += ` M${labelX} ${labelY + option.h}`;
          line += verticalLink(labelX, port, (rowBottom + ey) / 2, ey);
        } else {
          line += verticalLink(b.x, port, (sy + ey) / 2, ey);
        }
        path.setAttribute('d', line);
        path.setAttribute('class',`wire ${option.edge.kind==='unlock'?'unlock':option.edge.inferred?'inferred':''}`);
        path.setAttribute('marker-end','url(#arrow)');
        path.dataset.from=b.id;path.dataset.to=c.id;
        const edgeId = edgeIndex.get(option.edge);
        path.dataset.edge = edgeId;
        if (activeRoute) path.classList.add(activeRoute.edges.has(edgeId) ? 'route-wire' : 'route-muted');
        links.append(path);
      });
    }
  }

  function layout() {
    links.replaceChildren();
    roots.forEach(b => measure(b, true));
    const trees = roots.map(b => {
      const regions = arrange(b, 0);
      const left = Math.min(...regions.map(r => r.left));
      const right = Math.max(...regions.map(r => r.right));
      const height = Math.max(...regions.map(r => r.bottom));
      return {b, left, right, height};
    });
    const width = Math.max(...trees.map(tree => tree.right - tree.left)) + 2 * canvasPadding;
    let top = canvasPadding;
    for (const {b, left, right, height} of trees) {
      place(b, (width - left - right) / 2, top);
      top += height + 72;
    }
    dimensions = {width, height: top - 72 + canvasPadding};
    world.style.width = width + 'px';
    world.style.height = dimensions.height + 'px';
    svg.setAttribute('width', width);
    svg.setAttribute('height', dimensions.height);
  }
  function cameraDraw(){world.style.transform=`translate(${camera.x}px,${camera.y}px) scale(${camera.scale})`;board.style.backgroundPosition=`${camera.x}px ${camera.y}px`;scale.textContent=Math.round(camera.scale*100)+'%';}
  function fit(){camera.scale=Math.min(1,(board.clientWidth-32)/dimensions.width);camera.x=(board.clientWidth-dimensions.width*camera.scale)/2;camera.y=16;cameraDraw();}
  function zoom(r){const z=Math.max(.03,Math.min(3,camera.scale*r));const x=board.clientWidth/2,y=board.clientHeight/2;camera.x=x-(x-camera.x)*z/camera.scale;camera.y=y-(y-camera.y)*z/camera.scale;camera.scale=z;cameraDraw();}
  function reveal(b){if(b.element.hidden)return;const x=camera.x+b.x*camera.scale,y=camera.y+b.y*camera.scale,w=b.element.offsetWidth*camera.scale;if(x-w/2<8||x+w/2>board.clientWidth-8)camera.x=board.clientWidth/2-b.x*camera.scale;if(y<8||y+b.h*camera.scale>board.clientHeight-8)camera.y=40-b.y*camera.scale;cameraDraw();}
  function navigate(id){const b=entries.get(id);for(let p=b;p;p=p.parent)open.add(p.id);layout();camera.x=board.clientWidth/2-b.x*camera.scale;camera.y=60-b.y*camera.scale;cameraDraw();b.control.focus({preventScroll:true});showDetail(b);}
  function activate(b){
    const route = owners.get(b.id);
    if (route && route !== activeRoute) {
      selectRoute(route);
      camera.scale = Math.max(.85, camera.scale);
      navigate(b.id);
      return;
    }
    if(b.reference){navigate(b.id);return;}showDetail(b);if(!b.children.length)return;const {x,y}=b;open.has(b.id)?open.delete(b.id):open.add(b.id);layout();camera.x+=(x-b.x)*camera.scale;camera.y+=(y-b.y)*camera.scale;cameraDraw();}
  function download(name, value, type) {
    if (window.GalGearRoute?.export) {
      return window.GalGearRoute.export(type === 'application/json' ? 'json' : 'draft', value);
    }
    const link = el('a');
    link.href = URL.createObjectURL(new Blob([value], {type}));
    link.download = name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }
  const actions={expand(){entries.forEach((_,id)=>open.add(id));layout();fit();},collapse(){initial();layout();fit();},in(){zoom(1.2);},out(){zoom(1/1.2);},fit,theme(){document.documentElement.dataset.theme=document.documentElement.dataset.theme==='dark'?'light':'dark';},json(){const {ui,...route}=data;download('route-map-data.json',JSON.stringify(route,null,2),'application/json');},draft(){download('route-guide.txt',[data.title,t.draftNote,...data.nodes.flatMap(n=>[`\n[${n.id}] ${n.title}`, ...data.edges.filter(e=>e.from===n.id).map(e=>`${[edgeText(e), `→ [${e.to}]`].filter(Boolean).join(' ')}${e.inferred?' ('+t.inferred+')':''}`)]),...data.validation,...data.uncertainties].join('\n'),'text/plain');}};
  const toolbar=document.getElementById('toolbar');for(const key of Object.keys(actions)){const b=el('button','',t[key]);b.type='button';b.dataset.action=key;b.addEventListener('click',actions[key]);toolbar.append(b);}const scale=el('span','');toolbar.append(scale);
  function selectRoute(route) {
    activeRoute = route;
    picker.value = route?.entry || '';
    guidePanel.replaceChildren();
    guidePanel.hidden = !route;
    if (route) {
      // 展开真实节点及其树形引用的父链，使跨分支合流后的走法也能看见。
      for (const id of route.nodes) {
        for (let b = entries.get(id); b; b = b.parent) open.add(b.id);
      }
      if (route.prerequisite) guidePanel.append(el('p', 'route-prerequisite', route.prerequisite));
      if (route.choices?.length) {
        const summary = el('summary', '', t.entryChoices);
        const steps = el('ol', 'route-steps');
        for (const index of route.choices) {
          const edge = data.edges[index];
          const item = el('li');
          const jump = el('button', '', edge.label);
          jump.type = 'button';
          jump.addEventListener('click', () => navigate(edge.from));
          item.append(jump);
          steps.append(item);
        }
        const details = el('details');
        details.open = true;
        details.append(summary, steps);
        guidePanel.append(details);
      } else if (!route.guided) {
        guidePanel.append(el('p', '', t.possiblePaths));
      } else if (!route.prerequisite) {
        guidePanel.hidden = true;
      }
    }
    layout();
  }
  if (routes.length) {
    picker = el('select', 'route-picker');
    picker.setAttribute('aria-label', t.routeFilter);
    picker.add(new Option(t.allRoutes, ''));
    for (const route of routes) picker.add(new Option(route.name, route.entry));
    picker.addEventListener('change', () => {
      const route = routes.find(r => r.entry === picker.value);
      selectRoute(route);
      if (route) {
        camera.scale = Math.max(.85, camera.scale);
        navigate(route.choices?.length ? data.edges[route.choices[0]].from : route.entry);
      }
    });
    toolbar.prepend(picker);
    guidePanel = el('section', 'route-guide');
    guidePanel.hidden = true;
    toolbar.after(guidePanel);
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && activeRoute) {
        selectRoute(undefined);
        picker.focus();
      }
    });
  }
  const host = window.GalGearRoute;
  if (host?.locate) {
    const button = el('button', '', host.locateLabel);
    button.type = 'button';
    button.dataset.action = 'locate';
    toolbar.prepend(button);
    const status = el('p', 'location-status');
    status.setAttribute('role', 'status');
    toolbar.after(status);
    let marker;
    button.addEventListener('click', async () => {
      if (button.disabled) return;
      button.disabled = true;
      if (marker) {
        marker.control.removeAttribute('aria-current');
        marker.badge.remove();
        if (selected?.control === marker.control) detail.textContent = '';
        marker = undefined;
        layout();
      }
      status.textContent = host.locating;
      try {
        const result = await host.locate();
        const target = entries.get(result.node);
        if (!target) throw new Error(host.mapChanged);
        if (activeRoute && !activeRoute.nodes.has(target.id)) selectRoute(undefined);
        const badge = el('span', 'location-badge', host.current);
        target.control.append(badge);
        target.control.setAttribute('aria-current', 'location');
        marker = {control: target.control, badge};
        camera.scale = Math.max(1, camera.scale);
        navigate(result.node);
        status.textContent = result.detail;
      } catch (error) {
        status.textContent = String(error);
      } finally {
        button.disabled = false;
      }
    });
  }
  board.addEventListener('pointerdown',e=>{if(e.button!==0||e.target.closest('button'))return;drag={id:e.pointerId,x:e.clientX-camera.x,y:e.clientY-camera.y};board.dataset.dragging='';board.setPointerCapture(e.pointerId);});
  board.addEventListener('pointermove',e=>{if(drag?.id===e.pointerId){camera.x=e.clientX-drag.x;camera.y=e.clientY-drag.y;cameraDraw();}});
  board.addEventListener('lostpointercapture',()=>{drag=null;delete board.dataset.dragging;});
  board.addEventListener('wheel',e=>{e.preventDefault();if(e.ctrlKey)zoom(e.deltaY<0?1.1:1/1.1);else{camera.x-=e.deltaX;camera.y-=e.deltaY;cameraDraw();}},{passive:false});
  board.addEventListener('keydown',e=>{if(e.target!==board)return;const delta={ArrowLeft:[60,0],ArrowRight:[-60,0],ArrowUp:[0,60],ArrowDown:[0,-60]}[e.key];if(delta){e.preventDefault();camera.x+=delta[0];camera.y+=delta[1];cameraDraw();}});
  document.documentElement.dataset.theme=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';
  initial();layout();fit();let width=board.clientWidth;new ResizeObserver(()=>{const old=width;width=board.clientWidth;const b=selected&&!selected.element.hidden?selected:roots[0];const x=b.x,y=b.y;layout();camera.x+=(width-old)/2+(x-b.x)*camera.scale;camera.y+=(y-b.y)*camera.scale;cameraDraw();}).observe(board);
  document.fonts.ready.then(()=>{layout();if(selected)reveal(selected);else fit();});
})();
