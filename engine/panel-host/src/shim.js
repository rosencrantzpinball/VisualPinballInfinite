// Tauri's internals as the app's screens use them (@tauri-apps/api 2), for the headset's panel (vpi-panel,
// PLAN decision 20), which puts this before the screens' own scripts: a command is a request to the panel
// (`/__vpi/ipc/<command>`, its arguments as JSON, its reply as JSON or what went wrong in words), events
// come from the panel (`window.__VPI_EMIT__`), and the window and webview are one, `main`. The plugins'
// commands for windows have nothing to do in the headset and answer nothing. CEF draws no pointer off
// screen, so the page draws its own.
(() => {
  const callbacks = new Map();
  let next = 1;
  /** Each event's listeners: their IDs and the callbacks they run. */
  const listeners = new Map();

  function transformCallback(callback, once = false) {
    const id = next++;
    callbacks.set(id, (data) => {
      if (once) callbacks.delete(id);
      if (callback) callback(data);
    });
    return id;
  }

  function runCallback(id, data) {
    const f = callbacks.get(id);
    if (f) f(data);
  }

  function emit(event, payload) {
    const of = listeners.get(event);
    if (!of) return;
    for (const [id, handler] of [...of]) runCallback(handler, { event, id, payload });
  }

  async function invoke(cmd, args = {}) {
    switch (cmd) {
      case 'plugin:event|listen': {
        const id = next++;
        if (!listeners.has(args.event)) listeners.set(args.event, new Map());
        listeners.get(args.event).set(id, args.handler);
        return id;
      }
      case 'plugin:event|unlisten':
        listeners.get(args.event)?.delete(args.eventId);
        return null;
      case 'plugin:event|emit':
      case 'plugin:event|emit_to':
        emit(args.event, args.payload);
        return null;
    }
    if (cmd.startsWith('plugin:')) return null;
    const reply = await fetch('/__vpi/ipc/' + encodeURIComponent(cmd), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(args ?? {}),
    });
    const text = await reply.text();
    if (!reply.ok) throw text;
    return text ? JSON.parse(text) : null;
  }

  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback,
    callbacks,
    // no file is served by its path: the screens show what the panel names
    convertFileSrc: (path) => path,
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    plugins: {},
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (event, id) => listeners.get(event)?.delete(id) };
  window.__VPI_EMIT__ = emit;
  window.__VPI_HEADSET__ = true;
  document.documentElement.classList.add('headset');

  // the pointer, where the controller points: a ring over everything, never in the way of a click
  function pointer() {
    const ring = document.createElement('div');
    ring.setAttribute('aria-hidden', 'true');
    ring.style.cssText =
      'position:fixed;left:0;top:0;width:18px;height:18px;margin:-9px 0 0 -9px;border-radius:50%;' +
      'border:2px solid #fff;box-shadow:0 0 0 2px rgba(0,0,0,.55),0 0 10px rgba(0,0,0,.4);' +
      'pointer-events:none;z-index:2147483647;display:none;transition:transform .08s';
    document.body.appendChild(ring);
    addEventListener('mousemove', (e) => {
      ring.style.display = 'block';
      ring.style.translate = `${e.clientX}px ${e.clientY}px`;
    }, true);
    addEventListener('mousedown', () => (ring.style.transform = 'scale(.7)'), true);
    addEventListener('mouseup', () => (ring.style.transform = ''), true);
    document.addEventListener('mouseleave', () => (ring.style.display = 'none'));
  }
  if (document.body) pointer();
  else addEventListener('DOMContentLoaded', pointer);
})();
