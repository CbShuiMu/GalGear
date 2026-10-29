// Home-specific presentation and shelf navigation; selection remains in Rust.
import {topLayer} from './navigation.js';

let selectedGame = null;
let observedShelf = null;
let shelfObserver = null;
let alignmentFrame = 0;

const reducedMotion = matchMedia('(prefers-reduced-motion:reduce)');
let detailTransition = null;

const coverFrame = (bounds, radius, shadow) => ({
  transform: `translate(${bounds.left}px, ${bounds.top}px)`,
  width: `${bounds.width}px`, height: `${bounds.height}px`,
  borderRadius: radius, boxShadow: shadow
});

function createCoverFlight(source) {
  const flight = source.cloneNode(true);
  flight.className = 'poster home-detail-flight';
  flight.setAttribute('aria-hidden', 'true');
  flight.inert = true;
  flight.style.background = getComputedStyle(source).background;
  flight.querySelector('img')?.removeAttribute('loading');
  flight.querySelectorAll('.cover-tools,.cover-edit-actions').forEach(node => node.remove());
  document.body.append(flight);
  return flight;
}

export function cancelHomeDetailTransition() {
  const transition = detailTransition;
  if (!transition) return;
  detailTransition = null;
  transition.animations.forEach(animation => animation.cancel());
  transition.flight?.remove();
}

export function captureHomeDetailCover(gameId) {
  if (reducedMotion.matches) return null;
  const source = document.getElementById(`home-game-${gameId}`)?.querySelector('.poster');
  if (!source) return null;
  const bounds = source.getBoundingClientRect();
  if (!bounds.width || !bounds.height) return null;
  return {source, bounds, gameId, radius: getComputedStyle(source).borderRadius};
}

export function syncHomeDetailTransition(origin) {
  const overlay = document.querySelector('.detail-overlay');
  if (detailTransition && (detailTransition.overlay !== overlay || reducedMotion.matches)) {
    cancelHomeDetailTransition();
  }
  if (!origin || !overlay?.animate || reducedMotion.matches ||
      overlay.dataset.detailOrigin !== 'home' || overlay.dataset.detailGame !== origin.gameId) return;
  cancelHomeDetailTransition();
  const transition = {overlay, animations: [], flight: null};
  detailTransition = transition;
  const animate = (node, frames, duration, delay = 0) => {
    const animation = node.animate(frames, {
      duration, delay, easing: 'cubic-bezier(.22,.75,.2,1)', fill: 'both'
    });
    transition.animations.push(animation);
  };
  const home = origin.source.closest('.home-page');
  if (home) animate(home, [{opacity: 1}, {opacity: 0}], 260);
  animate(overlay, [{opacity: 0}, {opacity: 1}], 440);
  const reveal = (selector, duration, delay) => {
    overlay.querySelectorAll(selector).forEach((node, index) => {
      animate(node, [{opacity: 0}, {opacity: 1}], duration, delay + Math.min(index, 3) * 30);
    });
  };
  reveal('.detail-back-row', 340, 60);
  reveal('.detail-summary', 420, 90);
  reveal('.reading-card', 440, 130);
  reveal('.auxiliary-column > .detail-card', 400, 160);

  const cover = overlay.querySelector('.detail-cover');
  const image = origin.source.querySelector('img');
  // A failed or still-loading image uses the normal fade, never a blank flying poster.
  if (cover && (!image || (image.complete && image.naturalWidth))) {
    const target = cover.getBoundingClientRect();
    if (target.width && target.height) {
      const flight = createCoverFlight(origin.source);
      const style = getComputedStyle(cover);
      // Keep the image's aspect ratio while the frame changes shape and crops to the detail cover.
      transition.flight = flight;
      animate(flight, [coverFrame(origin.bounds, origin.radius, 'none'),
        coverFrame(target, style.borderRadius, style.boxShadow)], 560);
      animate(origin.source, [{opacity: 0}, {opacity: 0}], 560);
      animate(cover, [{opacity: 0}, {opacity: 0}], 560);
    }
  }
  Promise.all(transition.animations.map(animation => animation.finished.catch(() => {}))).then(() => {
    if (detailTransition === transition) cancelHomeDetailTransition();
  });
}

export function returnHomeDetail(overlay) {
  const target = document.getElementById(`home-game-${overlay.dataset.detailGame}`)?.querySelector('.poster');
  const home = target?.closest('.home-page') || document.querySelector('.home-page');
  const cover = overlay.querySelector('.detail-cover');
  const opening = detailTransition?.overlay === overlay ? detailTransition : null;
  // Reversing mid-entry starts from the visible cover, without snapping to its destination.
  const source = opening?.flight || cover;
  const from = source?.getBoundingClientRect();
  const to = target?.getBoundingClientRect();
  const sourceStyle = source && getComputedStyle(source);
  const start = from && coverFrame(from, sourceStyle.borderRadius, sourceStyle.boxShadow);
  const homeOpacity = opening && home ? getComputedStyle(home).opacity : 0;
  const overlayOpacity = getComputedStyle(overlay).opacity;
  const content = [...overlay.querySelectorAll('.detail-back-row,.detail-summary,.reading-card,.auxiliary-column > .detail-card,.detail-back-top')]
    .map(node => ({node, opacity: getComputedStyle(node).opacity}));
  const image = source?.querySelector('img');
  const viewport = overlay.querySelector('.detail-scroll')?.getBoundingClientRect();
  // A scrolled-out or unavailable cover should fade normally, not fly through clipped content.
  const visible = from && (!viewport || (from.top >= viewport.top && from.bottom <= viewport.bottom));
  const flight = visible && from.width && from.height && to?.width && to.height &&
    (!image || (image.complete && image.naturalWidth)) ? createCoverFlight(source) : null;
  cancelHomeDetailTransition();
  const transition = {overlay, animations: [], flight};
  detailTransition = transition;
  const animate = (node, frames, duration, delay = 0) => {
    transition.animations.push(node.animate(frames, {
      duration, delay, easing: 'cubic-bezier(.22,.75,.2,1)', fill: 'both'
    }));
  };
  if (home) animate(home, [{opacity: homeOpacity}, {opacity: 1}], 440);
  animate(overlay, [{opacity: overlayOpacity}, {opacity: 0}], 440);
  content.forEach(({node, opacity}) => animate(node, [{opacity}, {opacity: 0}], 260));
  if (flight) {
    const style = getComputedStyle(target);
    animate(flight, [start, coverFrame(to, style.borderRadius, style.boxShadow)], 560);
    animate(target, [{opacity: 0}, {opacity: 0}], 560);
    if (cover) animate(cover, [{opacity: 0}, {opacity: 0}], 560);
  }
  return Promise.all(transition.animations.map(animation => animation.finished.catch(() => {}))).then(() => {
    if (detailTransition === transition) cancelHomeDetailTransition();
  });
}

window.addEventListener('resize', cancelHomeDetailTransition);
document.addEventListener('scroll', event => {
  if (event.target.matches?.('.detail-scroll')) cancelHomeDetailTransition();
}, true);
reducedMotion.addEventListener('change', cancelHomeDetailTransition);

function updateShelfTail(shelf) {
  const last = shelf.lastElementChild;
  if (last?.matches('.home-game')) {
    shelf.style.setProperty('--home-last-width', `${last.getBoundingClientRect().width}px`);
  }
}

function alignSelection(game, animate) {
  cancelAnimationFrame(alignmentFrame);
  alignmentFrame = 0;
  if (!game) return;
  const shelf = game.parentElement;
  const from = shelf.scrollLeft;
  const started = performance.now();
  const duration = animate && !matchMedia('(prefers-reduced-motion:reduce)').matches
    ? Number(getComputedStyle(shelf).getPropertyValue('--home-motion-ms')) : 0;
  const step = now => {
    alignmentFrame = 0;
    if (!game.isConnected || !game.classList.contains('selected')) return;
    updateShelfTail(shelf);
    // Earlier posters shrink during the same transition, so follow their live
    // positions rather than scrolling to a stale offset and snapping at the end.
    const inset = parseFloat(getComputedStyle(shelf).paddingLeft) || 0;
    const target = shelf.scrollLeft + game.getBoundingClientRect().left - shelf.getBoundingClientRect().left - inset;
    const progress = duration ? Math.min(1, (now - started) / duration) : 1;
    shelf.scrollLeft = from + (target - from) * (1 - (1 - progress) ** 3);
    if (progress < 1) alignmentFrame = requestAnimationFrame(step);
  };
  if (duration) alignmentFrame = requestAnimationFrame(step);
  else step(started);
}

export function syncHomePresentation() {
  const image = document.querySelector('.home-backdrop-cover');
  if (image?.complete && image.naturalWidth && image.naturalHeight) {
    image.parentElement.dataset.layout = image.naturalWidth < image.naturalHeight ? 'portrait' : 'landscape';
  }
  const shelf = document.querySelector('.home-shelf');
  if (shelf !== observedShelf) {
    shelfObserver?.disconnect();
    observedShelf = shelf;
    selectedGame = null;
    if (shelf) shelfObserver?.observe(shelf);
  }
  const game = shelf?.querySelector('.home-game.selected') || null;
  if (game !== selectedGame) {
    const animate = !!selectedGame?.isConnected;
    selectedGame = game;
    alignSelection(game, animate);
  }
  if (shelf) updateShelfTail(shelf);
}

export function installHomeNavigation(dispatch, ready) {
  const select = game => {
    // Focus can be restored during a Rust render, so dispatch after that borrow ends.
    queueMicrotask(() => {
      if (ready() && game?.isConnected && document.activeElement === game) {
        if (game.getAttribute('aria-pressed') !== 'true') dispatch('home-select', game.dataset.value);
      }
    });
  };
  shelfObserver = new ResizeObserver(() => {
    // Responsive heights also transition; follow the moving offset until they settle.
    alignSelection(selectedGame, true);
  });
  if (observedShelf) shelfObserver.observe(observedShelf);
  document.addEventListener('load', event => {
    if (event.target.matches?.('.home-backdrop-cover')) syncHomePresentation();
    if (event.target.matches?.('.home-game img') && !alignmentFrame) alignSelection(selectedGame, false);
  }, true);
  document.addEventListener('focusin', event => {
    const game = event.target.closest?.('.home-game');
    if (game) select(game);
  });
  // Pointer presses must not transfer focus, which is used by keyboard/gamepad selection.
  document.addEventListener('pointerdown', event => {
    if (event.target.closest?.('.home-game')) event.preventDefault();
  });
  document.addEventListener('click', event => {
    if (event.detail > 0 && event.target.closest?.('.home-game')) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }, true);

  const activeHome = () => ready() && topLayer()?.matches('.app-shell') &&
    document.querySelector('.home-page:not([inert])') && !document.querySelector('.home-page')?.closest('[inert]');
  const move = direction => {
    const games = [...document.querySelectorAll('.home-shelf .home-game')];
    // Rapid wheel events can arrive before the focus-triggered render finishes.
    const focused = games.indexOf(document.activeElement);
    const index = Math.max(0, focused < 0 ? games.indexOf(selectedGame) : focused);
    const next = direction === 'first' ? 0 : direction === 'last' ? games.length - 1
      : Math.max(0, Math.min(games.length - 1, index + direction));
    games[next]?.focus({preventScroll:true});
  };
  document.addEventListener('keydown', event => {
    if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || !activeHome()) return;
    if (event.target.closest?.('input,textarea,select,[contenteditable="true"]')) return;
    const direction = {ArrowLeft:-1, ArrowRight:1, Home:'first', End:'last'}[event.key];
    if (direction === undefined) return;
    event.preventDefault();
    move(direction);
  });
  let wheelDelta = 0, lastWheelEvent = -Infinity;
  document.addEventListener('wheel', event => {
    if (!activeHome() || !event.target.closest?.('.home-page')) return;
    event.preventDefault();
    const shelf = document.querySelector('.home-shelf');
    if (!shelf) return;
    const now = performance.now();
    const scale = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? shelf.clientWidth : 1;
    const delta = (event.deltaY || event.deltaX) * scale;
    const direction = Math.sign(delta);
    if (!direction) return;
    if (now - lastWheelEvent > 200 || direction !== Math.sign(wheelDelta)) wheelDelta = 0;
    lastWheelEvent = now;
    wheelDelta += delta;
    if (Math.abs(wheelDelta) < 40) return;
    // Consume each wheel step immediately; a cooldown drops fast consecutive notches.
    move(direction);
    wheelDelta = 0;
  }, {passive:false});
}
