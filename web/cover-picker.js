// 尺寸来自实际加载的原图；加载失败或尚未加载时不显示占位尺寸。
function syncResolution(image) {
  const badge = image.parentElement.querySelector('.cover-resolution');
  const ready = image.complete && image.naturalWidth > 0 && image.naturalHeight > 0;
  badge.textContent = ready ? `${image.naturalWidth} × ${image.naturalHeight}` : '';
  badge.hidden = !ready;
}

export function syncCoverResolutions() {
  document.querySelectorAll('.cover-option-image img').forEach(syncResolution);
}

export function installCoverResolutions() {
  for (const type of ['load', 'error']) {
    document.addEventListener(type, event => {
      if (event.target.matches?.('.cover-option-image img')) syncResolution(event.target);
    }, true);
  }
}
