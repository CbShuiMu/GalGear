// 在页面脚本和 WASM 初始化之前安装；不记录参数或成功响应正文。
(() => {
  const internal = window.__TAURI_INTERNALS__;
  const invoke = internal.invoke.bind(internal);
  const page = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  let sequence = 0;
  let lastView;
  let cancelFrames;
  const text = value => {
    if (value instanceof Error) return `${value.message}\n${value.stack || ''}`;
    if (value && typeof value === 'object' && typeof value.key === 'string') {
      return JSON.stringify({key: value.key, args: value.args});
    }
    return typeof value === 'string' ? value : 'Non-text error';
  };
  // 日志通道失效不能再次走日志通道，也不能使原业务请求变成失败。
  const report = event => {
    try { return Promise.resolve(invoke('record_diagnostic', {event})).catch(() => {}); }
    catch { return Promise.resolve(); }
  };
  const stopWatchingFrames = () => {
    cancelFrames?.();
    cancelFrames = undefined;
  };
  const viewApplied = (scrollKey, overlay) => {
    const page = scrollKey?.split(':', 1)[0] || 'boot';
    if (lastView?.page === page && lastView.overlay === overlay) return;
    stopWatchingFrames();
    const view = {sequence: (lastView?.sequence ?? 0) + 1, page, overlay};
    lastView = view;
    const visible = document.visibilityState === 'visible';
    void report({kind: 'view', visible, view: {...view, phase: 'applied'}});
    if (!visible) return;

    const start = performance.now();
    const reportFrames = phase => {
      if (document.visibilityState === 'visible') {
        void report({kind: 'view', visible: true, view: {...view, phase}, elapsed_ms: Math.round(performance.now() - start)});
      }
    };
    // 两次帧回调只证明浏览器仍在处理帧，无法确认屏幕上呈现的像素。
    const timeout = window.setTimeout(() => reportFrames('frames_delayed'), 2000);
    let frame = window.requestAnimationFrame(() => {
      frame = window.requestAnimationFrame(() => {
        stopWatchingFrames();
        reportFrames('frames');
      });
    });
    cancelFrames = () => {
      window.clearTimeout(timeout);
      window.cancelAnimationFrame(frame);
    };
  };
  document.addEventListener('visibilitychange', () => {
    const visible = document.visibilityState === 'visible';
    if (!visible) stopWatchingFrames();
    void report({kind: 'visibility', visible});
  });
  window.GalGearDiagnostics = {
    error: error => report({kind: 'javascript_error', detail: text(error)}),
    panic: message => report({kind: 'wasm_panic', detail: `${message}\n${new Error().stack || ''}`}),
    ready: () => report({kind: 'ready', detail: navigator.userAgent}),
    viewApplied,
  };
  window.GalGearDiagnostics.invoke = async (command, args, options) => {
    if (command === 'record_diagnostic' || command.startsWith('plugin:')) return invoke(command, args, options);
    const request = `${page}-${++sequence}`;
    const start = performance.now();
    const headers = new Headers(options?.headers);
    headers.set('x-galgear-request', request);
    let result;
    try {
      result = await invoke(command, args, {...options, headers});
    } catch (error) {
      void report({kind: 'command_error', request, command, elapsed_ms: Math.round(performance.now() - start), detail: text(error)});
      throw error;
    }
    void report({kind: 'command_ok', request, command, elapsed_ms: Math.round(performance.now() - start)});
    return result;
  };
  window.addEventListener('error', event => {
    const detail = event.error ? text(event.error)
      : `${event.message || 'Resource load failed'} at ${event.filename || event.target?.src || ''}:${event.lineno || 0}:${event.colno || 0}`;
    void report({kind: 'javascript_error', detail});
  }, true);
  window.addEventListener('unhandledrejection', event => {
    void report({kind: 'javascript_error', detail: text(event.reason)});
  });
})();
