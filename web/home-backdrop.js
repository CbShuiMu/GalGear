// Presentation only: Rust owns the selected cover URL; this module owns its temporary layers.
const reducedMotion = matchMedia('(prefers-reduced-motion:reduce)');
let backdrop = null;

function finishTransition(state) {
  const transition = state.transition;
  if (!transition) return;
  state.transition = null;
  transition.previous?.remove();
  transition.animations.forEach(animation => animation.cancel());
}

function showLatest(state) {
  const request = state.request;
  if (backdrop !== state || !state.host.isConnected || state.transition ||
      !request?.ready || request.src === state.src) return;
  const previous = state.layer;
  const next = request.layer;
  state.src = request.src;
  state.layer = next;
  if (next) state.host.append(next);
  if (reducedMotion.matches || !state.host.animate) {
    previous?.remove();
    return;
  }
  const distance = request.direction * Math.min(56, state.host.clientWidth * .045);
  const options = {duration: 440, easing: 'cubic-bezier(.22,.75,.2,1)', fill: 'both'};
  const animations = [];
  if (previous) animations.push(previous.animate([
    {opacity: 1, transform: 'translateX(0)'},
    {opacity: 0, transform: `translateX(${-distance}px)`}
  ], options));
  if (next) animations.push(next.animate([
    {opacity: 0, transform: `translateX(${distance}px)`},
    {opacity: 1, transform: 'translateX(0)'}
  ], options));
  const transition = {previous, animations};
  state.transition = transition;
  Promise.all(animations.map(animation => animation.finished.catch(() => {}))).then(() => {
    if (backdrop !== state || state.transition !== transition) return;
    finishTransition(state);
    // Keep the current fade continuous, then skip intermediate selections and show only the latest.
    showLatest(state);
  });
}

async function loadCover(state, request) {
  const image = new Image();
  state.loadingImage = image;
  image.alt = '';
  image.decoding = 'async';
  image.src = request.src;
  try {
    await image.decode();
    if (backdrop !== state || state.request !== request || !state.host.isConnected) return;
    const layer = document.createElement('div');
    layer.className = 'home-backdrop-layer';
    layer.dataset.layout = image.naturalWidth < image.naturalHeight ? 'portrait' : 'landscape';
    for (const name of layer.dataset.layout === 'portrait' ? ['bleed', 'soft'] : []) {
      const copy = image.cloneNode();
      copy.className = `home-backdrop-${name}`;
      layer.append(copy);
    }
    image.className = 'home-backdrop-cover';
    layer.append(image);
    request.layer = layer;
  } catch {
    // An unavailable cover fades to the page surface, never leaves another game's artwork behind.
  } finally {
    state.loadingImage = null;
    if (backdrop === state && state.request !== request) loadLatest(state);
  }
  if (backdrop !== state || state.request !== request || !state.host.isConnected) return;
  request.ready = true;
  showLatest(state);
}

function loadLatest(state) {
  const request = state.request;
  if (state.host.isConnected && !state.loadingImage && request?.src && !request.ready && request.src !== state.src) {
    void loadCover(state, request);
  }
}

export function syncHomeBackdrop(host, direction) {
  if (host !== backdrop?.host) {
    if (backdrop) {
      finishTransition(backdrop);
      backdrop.layer?.remove();
      backdrop.loadingImage?.removeAttribute('src');
    }
    backdrop = host ? {host, src: '', layer: null, request: null, transition: null, loadingImage: null} : null;
  }
  if (!backdrop) return;
  const state = backdrop;
  const src = host.dataset.cover || '';
  if (src === state.request?.src) return;
  const request = {src, direction, ready: !src, layer: null};
  state.request = request;
  if (src === state.src) return;
  // Serial decoding bounds memory during rapid navigation, retaining only the latest queued selection.
  if (src) loadLatest(state);
  else showLatest(state);
}

reducedMotion.addEventListener('change', () => {
  if (!backdrop || !reducedMotion.matches) return;
  finishTransition(backdrop);
  showLatest(backdrop);
});
