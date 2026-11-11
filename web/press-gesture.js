// 卡片和标签共用短按/长按分界，避免相同手势在不同内容上有不同手感。
export const LONG_PRESS_DELAY_MS = 380;

export function movedPastLongPressSlop(origin, event) {
  return Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 6;
}
