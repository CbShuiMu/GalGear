/* 桌面窗口由后端注入结构化数据，离线 HTML 使用同一套路线渲染器。 */
(() => {
  'use strict';
  const host = window.GalGearRoute;
  document.documentElement.lang = host.language;
  document.title = host.title;
  if (host.document.svg) {
    // 保留旧图的 SVG 标题提示和文本选择，沙盒不授予脚本和宿主访问权限。
    const frame = document.createElement('iframe');
    frame.className = 'legacy-route';
    frame.title = host.title;
    frame.setAttribute('sandbox', '');
    frame.srcdoc = host.document.svg;
    document.body.replaceChildren(frame);
    return;
  }
  const data = document.createElement('script');
  data.type = 'application/json';
  data.id = 'route-data';
  data.textContent = JSON.stringify(host.document.interactive);
  document.body.append(data);
  host.export = async (kind, contents) => {
    try {
      await window.__TAURI__.core.invoke('export_route', {kind, contents});
    } catch (error) {
      document.getElementById('detail').textContent = host.exportError.replace('{detail}', () => String(error));
    }
  };
})();
