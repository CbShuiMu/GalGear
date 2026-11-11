// Shared hit testing for card clicks, marquee selection and long-press reordering.
export function isCardScrollbar(event) {
  const tags = event.target.closest('.card-tags');
  if (!tags) return false;
  const bounds = tags.getBoundingClientRect();
  // Overlay scrollbars may not subtract from clientWidth. Reserve their thin hit area.
  const vertical = tags.scrollHeight > tags.clientHeight
    && event.clientX >= bounds.right - Math.max(8, tags.offsetWidth - tags.clientWidth);
  const horizontal = tags.scrollWidth > tags.clientWidth
    && event.clientY >= bounds.bottom - Math.max(8, tags.offsetHeight - tags.clientHeight);
  return vertical || horizontal;
}

export function libraryCardClickTarget(event) {
  if (isCardScrollbar(event)) return null;
  const action = event.target.closest('[data-action]');
  if (action) return action;
  // Tags intercept pointer events for scrolling, but share the cover's short-click action.
  return event.target.closest('.card-tags')?.closest('.game-card')?.querySelector('.game-face') ?? null;
}
