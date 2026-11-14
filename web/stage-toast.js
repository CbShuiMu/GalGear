// Only the two temporary text layers belong to JS; Rust owns the current stage.
const transitions = new WeakMap();
const active = new Set();
let reducedMotion;

function motionPreference() {
  if (!reducedMotion) {
    reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
    reducedMotion.addEventListener('change', () => {
      if (reducedMotion.matches) for (const node of [...active]) finish(node);
    });
  }
  return reducedMotion.matches;
}

function finish(node) {
  const transition = transitions.get(node);
  if (!transition) return;
  transitions.delete(node);
  active.delete(node);
  for (const animation of transition.animations) animation.cancel();
  transition.previous.remove();
}

export function reconcileStageToast(current, next) {
  const incoming = next.querySelector('.stage-toast-current');
  const visible = current.querySelector('.stage-toast-current');
  if (!incoming || !visible) return;
  const text = incoming.textContent;
  if (text === visible.textContent) return;
  // A fast update replaces the in-flight transition immediately, without a queue.
  finish(current);
  const previous = visible.cloneNode(true);
  visible.textContent = text;
  current.title = text;
  if (motionPreference() || next.dataset.immediate === 'true' || !visible.animate) return;
  previous.className = 'stage-toast-previous';
  previous.setAttribute('aria-hidden', 'true');
  current.append(previous);
  const options = {duration: 300, easing: 'cubic-bezier(.2,.7,.2,1)'};
  const animations = [
    previous.animate([{transform: 'translateY(0)', opacity: 1}, {transform: 'translateY(-100%)', opacity: 0}], options),
    visible.animate([{transform: 'translateY(100%)', opacity: 0}, {transform: 'translateY(0)', opacity: 1}], options),
  ];
  const transition = {previous, animations};
  transitions.set(current, transition);
  active.add(current);
  Promise.all(animations.map(animation => animation.finished)).then(() => {
    if (transitions.get(current) === transition) finish(current);
  }).catch(() => {}); // Cancellation is expected when a newer stage arrives.
}
