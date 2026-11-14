/* 离线视图只消费宿主已校验的图；共享子树和循环转换为引用，不补造业务边。 */
(() => {
  'use strict';
  const data = JSON.parse(document.getElementById('route-data').textContent);
  const t = data.ui;
  const board = document.getElementById('board'), world = document.getElementById('world');
  const layer = document.getElementById('nodes'), svg = document.getElementById('wires');
  const links = document.getElementById('links'), detail = document.getElementById('detail');
  const nodes = new Map(data.nodes.map(n => [n.id, n]));
  const outgoing = new Map(data.nodes.map(n => [n.id, data.edges.filter(e => e.from === n.id)]));
  const entries = new Map(), branches = [], open = new Set(), roots = [];
  const camera = {x:0,y:16,scale:1};
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
  function showDetail(b){selected=b; detail.textContent=[b.node.title,b.incoming.label,b.incoming.condition,b.incoming.pc,b.node.pc,b.incoming.evidence,b.node.evidence].filter(Boolean).join('\n');}
  function create(id, incoming={}, parent=null){
    const node=nodes.get(id), reference=entries.has(id);
    const b={id,node,incoming,parent,reference,children:[]}; branches.push(b);
    b.element=el('div',`node ${node.type} ${reference?'reference':''}`);
    b.control=el('button','');b.control.type='button';b.control.dataset[reference?'jump':'node']=id;
    b.icon=el('span');b.icon.setAttribute('aria-hidden','true');
    b.control.append(b.icon,document.createTextNode(' '+(node.type==='extra'?'EXTRA · ':node.type==='grand'?'GRAND · ':'')+node.title));
    b.element.append(b.control);layer.append(b.element);
    const label=[incoming.label,incoming.condition].filter(Boolean).join(' · ');
    if(label){b.label=el('span',`edge-label ${incoming.kind==='unlock'?'unlock':''}`,label);b.label.id='option-'+branches.length;layer.append(b.label);b.control.setAttribute('aria-describedby',b.label.id);}
    if(!reference){entries.set(id,b);for(const edge of outgoing.get(id))b.children.push(create(edge.to,edge,b));}
    b.control.addEventListener('click',()=>activate(b));
    b.control.addEventListener('focus',()=>reveal(b));
    return b;
  }
  // Start roots first. Any disconnected evidence stays a separate tree, with an explicit validation warning.
  for(const node of data.nodes.filter(n=>n.type==='start'))if(!entries.has(node.id))roots.push(create(node.id));
  for(const node of data.nodes)if(!entries.has(node.id))roots.push(create(node.id));
  function initial(){open.clear();const depth=board.clientWidth<540?2:4;for(const b of entries.values()){let d=0;for(let p=b.parent;p;p=p.parent)d++;if(d<depth)open.add(b.id);}}
  function measure(b, visible){
    b.element.hidden=!visible;if(b.label)b.label.hidden=!visible;
    const expanded=visible&&open.has(b.id)&&!b.reference;
    b.icon.textContent=b.reference?'↗':b.children.length?(expanded?'▾':'▸'):b.node.type==='start'?'▷':b.node.type==='save'?'◇':'⚑';
    if(b.children.length)b.control.setAttribute('aria-expanded',String(expanded));
    for(const c of b.children)measure(c,expanded);
    if(!visible)return;
    b.h=b.element.offsetHeight;b.lh=b.label?.offsetHeight||0;
    b.group=expanded?b.children.reduce((s,c)=>s+c.w,0)+Math.max(0,b.children.length-1)*28:0;
    b.w=Math.max(b.element.offsetWidth,b.label?.offsetWidth||0,b.group);
  }
  function place(b,left,y){
    b.x=left+b.w/2;b.y=y;b.element.style.left=(b.x-b.element.offsetWidth/2)+'px';b.element.style.top=y+'px';
    let bottom=y+b.h;
    if(!open.has(b.id)||b.reference||!b.children.length)return bottom;
    const labelY=bottom+28, childY=labelY+Math.max(...b.children.map(c=>c.lh))+24;
    let next=b.x-b.group/2;
    for(const c of b.children){bottom=Math.max(bottom,place(c,next,childY));next+=c.w+28;
      if(c.label){c.label.style.left=(c.x-c.label.offsetWidth/2)+'px';c.label.style.top=labelY+'px';}
      const path=document.createElementNS('http://www.w3.org/2000/svg','path');
      const sy=y+b.h, ey=childY-3, bend=(sy+labelY)/2;
      path.setAttribute('d',`M${b.x} ${sy} C${b.x} ${bend} ${c.x} ${bend} ${c.x} ${labelY}${c.label?` M${c.x} ${labelY+c.lh}`:''} L${c.x} ${ey}`);
      path.setAttribute('class',`wire ${c.incoming.kind==='unlock'?'unlock':c.incoming.inferred?'inferred':''}`);path.setAttribute('marker-end','url(#arrow)');
      path.dataset.from=b.id;path.dataset.to=c.id;links.append(path);
    }return bottom;
  }
  function layout(){links.replaceChildren();roots.forEach(b=>measure(b,true));const width=Math.max(...roots.map(b=>b.w))+64;let y=32;for(const b of roots)y=place(b,(width-b.w)/2,y)+72;dimensions={width,height:y-40};world.style.width=width+'px';world.style.height=dimensions.height+'px';svg.setAttribute('width',width);svg.setAttribute('height',dimensions.height);}
  function cameraDraw(){world.style.transform=`translate(${camera.x}px,${camera.y}px) scale(${camera.scale})`;board.style.backgroundPosition=`${camera.x}px ${camera.y}px`;scale.textContent=Math.round(camera.scale*100)+'%';}
  function fit(){camera.scale=Math.min(1,(board.clientWidth-32)/dimensions.width);camera.x=(board.clientWidth-dimensions.width*camera.scale)/2;camera.y=16;cameraDraw();}
  function zoom(r){const z=Math.max(.03,Math.min(3,camera.scale*r));const x=board.clientWidth/2,y=board.clientHeight/2;camera.x=x-(x-camera.x)*z/camera.scale;camera.y=y-(y-camera.y)*z/camera.scale;camera.scale=z;cameraDraw();}
  function reveal(b){if(b.element.hidden)return;const x=camera.x+b.x*camera.scale,y=camera.y+b.y*camera.scale,w=b.element.offsetWidth*camera.scale;if(x-w/2<8||x+w/2>board.clientWidth-8)camera.x=board.clientWidth/2-b.x*camera.scale;if(y<8||y+b.h*camera.scale>board.clientHeight-8)camera.y=40-b.y*camera.scale;cameraDraw();}
  function navigate(id){const b=entries.get(id);for(let p=b;p;p=p.parent)open.add(p.id);layout();camera.x=board.clientWidth/2-b.x*camera.scale;camera.y=60-b.y*camera.scale;cameraDraw();b.control.focus({preventScroll:true});showDetail(b);}
  function activate(b){if(b.reference){navigate(b.id);return;}showDetail(b);if(!b.children.length)return;const {x,y}=b;open.has(b.id)?open.delete(b.id):open.add(b.id);layout();camera.x+=(x-b.x)*camera.scale;camera.y+=(y-b.y)*camera.scale;cameraDraw();}
  function download(name,value,type){const a=el('a');a.href=URL.createObjectURL(new Blob([value],{type}));a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000);}
  const actions={expand(){entries.forEach((_,id)=>open.add(id));layout();fit();},collapse(){initial();layout();fit();},in(){zoom(1.2);},out(){zoom(1/1.2);},fit,theme(){document.documentElement.dataset.theme=document.documentElement.dataset.theme==='dark'?'light':'dark';},json(){const {ui,...route}=data;download('route-map-data.json',JSON.stringify(route,null,2),'application/json');},draft(){download('route-guide.txt',[data.title,t.draftNote,...data.nodes.flatMap(n=>[`\n[${n.id}] ${n.title}`, ...data.edges.filter(e=>e.from===n.id).map(e=>`${e.label||t.next}${e.condition?' ['+e.condition+']':''} → [${e.to}]${e.inferred?' ('+t.inferred+')':''}`)]),...data.validation,...data.uncertainties].join('\n'),'text/plain');}};
  const toolbar=document.getElementById('toolbar');for(const key of Object.keys(actions)){const b=el('button','',t[key]);b.type='button';b.dataset.action=key;b.addEventListener('click',actions[key]);toolbar.append(b);}const scale=el('span','');toolbar.append(scale);
  board.addEventListener('pointerdown',e=>{if(e.button!==0||e.target.closest('button'))return;drag={id:e.pointerId,x:e.clientX-camera.x,y:e.clientY-camera.y};board.dataset.dragging='';board.setPointerCapture(e.pointerId);});
  board.addEventListener('pointermove',e=>{if(drag?.id===e.pointerId){camera.x=e.clientX-drag.x;camera.y=e.clientY-drag.y;cameraDraw();}});
  board.addEventListener('lostpointercapture',()=>{drag=null;delete board.dataset.dragging;});
  board.addEventListener('wheel',e=>{e.preventDefault();if(e.ctrlKey)zoom(e.deltaY<0?1.1:1/1.1);else{camera.x-=e.deltaX;camera.y-=e.deltaY;cameraDraw();}},{passive:false});
  board.addEventListener('keydown',e=>{if(e.target!==board)return;const delta={ArrowLeft:[60,0],ArrowRight:[-60,0],ArrowUp:[0,60],ArrowDown:[0,-60]}[e.key];if(delta){e.preventDefault();camera.x+=delta[0];camera.y+=delta[1];cameraDraw();}});
  document.documentElement.dataset.theme=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';
  initial();layout();fit();let width=board.clientWidth;new ResizeObserver(()=>{const old=width;width=board.clientWidth;const b=selected&&!selected.element.hidden?selected:roots[0];const x=b.x,y=b.y;layout();camera.x+=(width-old)/2+(x-b.x)*camera.scale;camera.y+=(y-b.y)*camera.scale;cameraDraw();}).observe(board);
  document.fonts.ready.then(()=>{layout();fit();});
})();
