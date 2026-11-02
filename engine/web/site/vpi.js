// Visual Pinball Infinite in the browser: the page that starts the player (vpi.wasm, engine/web) on
// one table and drives it. It fetches the table's files (manifest.json lists them) into the program's
// files, which WASI's calls reach: /table, read only, and /data, the player's settings and what the
// table keeps (its high scores), kept in this browser's local storage. Each animation frame it hands
// the program the keys, mouse, gamepads and touches, asks for a frame, and plays the sound that comes.
// On a phone or tablet the screen's halves are the flippers; its Report button (Test report before
// play) shows what the browser has and does, to copy and send.

import { Gpu } from './gpu.js';

const $ = (id) => document.getElementById(id);
const status = (text) => ($('status').textContent = text);
const log = (text) => console.log(text);
const params = new URLSearchParams(location.search);
// a phone or tablet (`?touch` or `?touch=0` to say otherwise): touch controls, fewer pixels, the upright view
const touchDevice = params.has('touch') ? params.get('touch') !== '0' : matchMedia('(pointer: coarse)').matches || navigator.maxTouchPoints > 1;
// the player's last lines, for the report
const lines = [];
const keep = (line) => {
  lines.push(line);
  if (lines.length > 80) lines.shift();
};

// ---- the program's files (WASI's preview 1, enough for the player) ----

const E = { SUCCESS: 0, ACCES: 2, BADF: 8, EXIST: 20, INVAL: 28, ISDIR: 31, NOENT: 44, NOSYS: 52, NOTDIR: 54, NOTEMPTY: 55 };
const FILETYPE = { CHAR: 2, DIR: 3, FILE: 4 };

class Dir {
  constructor() {
    this.children = new Map();
    this.mtime = BigInt(Date.now()) * 1000000n;
  }
}
class File {
  constructor(data, persist) {
    this.data = data;
    this.size = data.length;
    this.persist = persist;
    this.mtime = BigInt(Date.now()) * 1000000n;
  }
  grow(n) {
    if (n <= this.data.length) return;
    const d = new Uint8Array(Math.max(n, this.data.length * 2, 4096));
    d.set(this.data.subarray(0, this.size));
    this.data = d;
  }
  bytes() {
    return this.data.subarray(0, this.size);
  }
}

class Files {
  constructor(storeKey) {
    this.root = new Dir();
    this.storeKey = storeKey;
    this.dirty = false;
    this.saveTimer = 0;
  }
  // a path's parts, `.` and `..` resolved
  parts(path) {
    const out = [];
    for (const p of path.split('/')) {
      if (p === '' || p === '.') continue;
      if (p === '..') out.pop();
      else out.push(p);
    }
    return out;
  }
  lookup(parts) {
    let node = this.root;
    for (const p of parts) {
      if (!(node instanceof Dir)) return null;
      node = node.children.get(p);
      if (!node) return null;
    }
    return node;
  }
  parent(parts, create) {
    let node = this.root;
    for (const p of parts.slice(0, -1)) {
      let next = node.children.get(p);
      if (!next && create) node.children.set(p, (next = new Dir()));
      if (!(next instanceof Dir)) return null;
      node = next;
    }
    return node;
  }
  put(path, data, persist) {
    const parts = this.parts(path);
    this.parent(parts, true).children.set(parts[parts.length - 1], new File(data, persist));
  }
  mkdir(path) {
    const parts = this.parts(path);
    let node = this.root;
    for (const p of parts) {
      let next = node.children.get(p);
      if (!next) node.children.set(p, (next = new Dir()));
      node = next;
    }
  }
  // /data, from and to local storage: each file as base64, by its path
  load() {
    try {
      const saved = JSON.parse(localStorage.getItem(this.storeKey) || '{}');
      for (const [path, b64] of Object.entries(saved)) {
        const bin = atob(b64);
        const data = new Uint8Array(bin.length);
        for (let k = 0; k < bin.length; k++) data[k] = bin.charCodeAt(k);
        this.put(path, data, true);
      }
    } catch (e) {
      log(`the saved settings couldn't be read: ${e}`);
    }
  }
  changed() {
    this.dirty = true;
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => this.save(), 500);
  }
  save() {
    if (!this.dirty) return;
    this.dirty = false;
    const out = {};
    const walk = (dir, path) => {
      for (const [name, node] of dir.children) {
        const p = `${path}/${name}`;
        if (node instanceof Dir) walk(node, p);
        else if (node.persist) {
          let bin = '';
          const b = node.bytes();
          for (let k = 0; k < b.length; k += 0x8000) bin += String.fromCharCode.apply(null, b.subarray(k, k + 0x8000));
          out[p] = btoa(bin);
        }
      }
    };
    walk(this.root, '');
    try {
      localStorage.setItem(this.storeKey, JSON.stringify(out));
    } catch (e) {
      log(`the settings couldn't be saved in this browser: ${e}`);
    }
  }
}

class Wasi {
  constructor(files, env, out) {
    this.files = files;
    this.env = env;
    this.out = out; // (fd, text)
    this.fds = new Map([[3, { node: files.root, path: '/', pos: 0 }]]);
    this.next = 4;
    this.memory = null;
    this.lines = ['', '', ''];
    this.decoder = new TextDecoder();
    this.encoder = new TextEncoder();
  }
  get view() {
    return new DataView(this.memory.buffer);
  }
  get bytes() {
    return new Uint8Array(this.memory.buffer);
  }
  str(p, n) {
    return this.decoder.decode(new Uint8Array(this.memory.buffer, p, n));
  }
  // the node at `path` from the folder `fd` has open
  resolve(fd, path) {
    const base = this.fds.get(fd);
    if (!base || !(base.node instanceof Dir)) return null;
    return this.files.parts(`${base.path}/${path}`);
  }
  stat(node, at) {
    const v = this.view;
    const type = node instanceof Dir ? FILETYPE.DIR : FILETYPE.FILE;
    for (let k = 0; k < 64; k += 8) v.setBigUint64(at + k, 0n, true);
    v.setBigUint64(at + 8, BigInt(node.ino ??= Wasi.ino++), true);
    v.setUint8(at + 16, type);
    v.setBigUint64(at + 24, 1n, true);
    v.setBigUint64(at + 32, BigInt(node instanceof File ? node.size : 4096), true);
    v.setBigUint64(at + 40, node.mtime, true);
    v.setBigUint64(at + 48, node.mtime, true);
    v.setBigUint64(at + 56, node.mtime, true);
  }
  imports() {
    const w = this;
    return {
      args_get: () => E.SUCCESS,
      args_sizes_get: (argc, size) => {
        w.view.setUint32(argc, 0, true);
        w.view.setUint32(size, 0, true);
        return E.SUCCESS;
      },
      environ_sizes_get: (count, size) => {
        const all = Object.entries(w.env).map(([k, v]) => `${k}=${v}\0`);
        w.view.setUint32(count, all.length, true);
        w.view.setUint32(size, all.reduce((n, s) => n + w.encoder.encode(s).length, 0), true);
        return E.SUCCESS;
      },
      environ_get: (list, buf) => {
        let at = buf;
        Object.entries(w.env).forEach(([k, v], i) => {
          const b = w.encoder.encode(`${k}=${v}\0`);
          w.bytes.set(b, at);
          w.view.setUint32(list + 4 * i, at, true);
          at += b.length;
        });
        return E.SUCCESS;
      },
      clock_res_get: (id, res) => {
        w.view.setBigUint64(res, 1000n, true);
        return E.SUCCESS;
      },
      clock_time_get: (id, precision, time) => {
        const ns = id === 0 ? BigInt(Date.now()) * 1000000n : BigInt(Math.round(performance.now() * 1e6));
        w.view.setBigUint64(time, ns, true);
        return E.SUCCESS;
      },
      random_get: (buf, len) => {
        for (let k = 0; k < len; k += 65536) crypto.getRandomValues(new Uint8Array(w.memory.buffer, buf + k, Math.min(65536, len - k)));
        return E.SUCCESS;
      },
      fd_prestat_get: (fd, buf) => {
        if (fd !== 3) return E.BADF;
        w.view.setUint8(buf, 0);
        w.view.setUint32(buf + 4, 1, true);
        return E.SUCCESS;
      },
      fd_prestat_dir_name: (fd, path, len) => {
        if (fd !== 3) return E.BADF;
        w.bytes[path] = 47; // '/'
        return E.SUCCESS;
      },
      fd_fdstat_get: (fd, buf) => {
        const v = w.view;
        const f = w.fds.get(fd);
        const type = fd <= 2 ? FILETYPE.CHAR : !f ? -1 : f.node instanceof Dir ? FILETYPE.DIR : FILETYPE.FILE;
        if (type < 0) return E.BADF;
        v.setUint8(buf, type);
        v.setUint16(buf + 2, f?.append ? 1 : 0, true);
        v.setBigUint64(buf + 8, 0xffffffffffffffffn, true);
        v.setBigUint64(buf + 16, 0xffffffffffffffffn, true);
        return E.SUCCESS;
      },
      fd_fdstat_set_flags: () => E.SUCCESS,
      fd_filestat_get: (fd, buf) => {
        const f = w.fds.get(fd);
        if (!f) return fd <= 2 ? E.SUCCESS : E.BADF;
        w.stat(f.node, buf);
        return E.SUCCESS;
      },
      fd_filestat_set_size: (fd, size) => {
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof File)) return E.BADF;
        const n = Number(size);
        f.node.grow(n);
        if (n > f.node.size) f.node.data.fill(0, f.node.size, n);
        f.node.size = n;
        if (f.node.persist) w.files.changed();
        return E.SUCCESS;
      },
      path_open: (dirfd, dirflags, path, len, oflags, rights, inheriting, fdflags, out) => {
        const parts = w.resolve(dirfd, w.str(path, len));
        if (!parts) return E.BADF;
        let node = w.files.lookup(parts);
        const create = oflags & 1, directory = oflags & 2, exclusive = oflags & 4, truncate = oflags & 8;
        if (node && create && exclusive) return E.EXIST;
        if (!node) {
          if (!create) return E.NOENT;
          const parent = w.files.parent(parts, false);
          if (!parent) return E.NOENT;
          const persist = parts[0] === 'data';
          if (!persist) return E.ACCES; // the table's folder is only read
          node = new File(new Uint8Array(0), true);
          parent.children.set(parts[parts.length - 1], node);
          w.files.changed();
        }
        if (directory && !(node instanceof Dir)) return E.NOTDIR;
        if (node instanceof File && truncate) {
          if (!node.persist) return E.ACCES;
          node.size = 0;
          w.files.changed();
        }
        const fd = w.next++;
        w.fds.set(fd, { node, path: `/${parts.join('/')}`, pos: 0, append: (fdflags & 1) !== 0 });
        w.view.setUint32(out, fd, true);
        return E.SUCCESS;
      },
      fd_close: (fd) => {
        if (!w.fds.delete(fd)) return E.BADF;
        return E.SUCCESS;
      },
      fd_read: (fd, iovs, n, nread) => {
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof File)) return fd === 0 ? (w.view.setUint32(nread, 0, true), E.SUCCESS) : E.BADF;
        const v = w.view;
        const bytes = w.bytes;
        let total = 0;
        for (let k = 0; k < n; k++) {
          const p = v.getUint32(iovs + 8 * k, true);
          const l = v.getUint32(iovs + 8 * k + 4, true);
          const take = Math.max(0, Math.min(l, f.node.size - f.pos));
          bytes.set(f.node.data.subarray(f.pos, f.pos + take), p);
          f.pos += take;
          total += take;
          if (take < l) break;
        }
        v.setUint32(nread, total, true);
        return E.SUCCESS;
      },
      fd_pread: (fd, iovs, n, offset, nread) => {
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof File)) return E.BADF;
        const keep = f.pos;
        f.pos = Number(offset);
        const r = w.imports().fd_read(fd, iovs, n, nread);
        f.pos = keep;
        return r;
      },
      fd_write: (fd, iovs, n, nwritten) => {
        const v = w.view;
        let total = 0;
        if (fd === 1 || fd === 2) {
          for (let k = 0; k < n; k++) {
            const p = v.getUint32(iovs + 8 * k, true);
            const l = v.getUint32(iovs + 8 * k + 4, true);
            w.lines[fd] += w.str(p, l);
            total += l;
          }
          let nl;
          while ((nl = w.lines[fd].indexOf('\n')) >= 0) {
            w.out(fd, w.lines[fd].slice(0, nl));
            w.lines[fd] = w.lines[fd].slice(nl + 1);
          }
          v.setUint32(nwritten, total, true);
          return E.SUCCESS;
        }
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof File)) return E.BADF;
        if (!f.node.persist) return E.ACCES;
        if (f.append) f.pos = f.node.size;
        for (let k = 0; k < n; k++) {
          const p = v.getUint32(iovs + 8 * k, true);
          const l = v.getUint32(iovs + 8 * k + 4, true);
          f.node.grow(f.pos + l);
          f.node.data.set(new Uint8Array(w.memory.buffer, p, l), f.pos);
          f.pos += l;
          f.node.size = Math.max(f.node.size, f.pos);
          total += l;
        }
        f.node.mtime = BigInt(Date.now()) * 1000000n;
        w.files.changed();
        v.setUint32(nwritten, total, true);
        return E.SUCCESS;
      },
      fd_seek: (fd, offset, whence, out) => {
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof File)) return E.BADF;
        const base = whence === 0 ? 0 : whence === 1 ? f.pos : f.node.size;
        const pos = base + Number(offset);
        if (pos < 0) return E.INVAL;
        f.pos = pos;
        w.view.setBigUint64(out, BigInt(pos), true);
        return E.SUCCESS;
      },
      fd_tell: (fd, out) => {
        const f = w.fds.get(fd);
        if (!f) return E.BADF;
        w.view.setBigUint64(out, BigInt(f.pos), true);
        return E.SUCCESS;
      },
      fd_sync: () => E.SUCCESS,
      fd_datasync: () => E.SUCCESS,
      fd_readdir: (fd, buf, len, cookie, used) => {
        const f = w.fds.get(fd);
        if (!f || !(f.node instanceof Dir)) return E.NOTDIR;
        const v = w.view;
        const entries = [...f.node.children.entries()];
        let at = 0;
        for (let k = Number(cookie); k < entries.length && at < len; k++) {
          const [name, node] = entries[k];
          const nb = w.encoder.encode(name);
          const head = new DataView(new ArrayBuffer(24));
          head.setBigUint64(0, BigInt(k + 1), true);
          head.setBigUint64(8, BigInt((node.ino ??= Wasi.ino++)), true);
          head.setUint32(16, nb.length, true);
          head.setUint8(20, node instanceof Dir ? FILETYPE.DIR : FILETYPE.FILE);
          const entry = new Uint8Array(24 + nb.length);
          entry.set(new Uint8Array(head.buffer), 0);
          entry.set(nb, 24);
          const take = Math.min(entry.length, len - at);
          w.bytes.set(entry.subarray(0, take), buf + at);
          at += take;
        }
        v.setUint32(used, at, true);
        return E.SUCCESS;
      },
      path_filestat_get: (fd, flags, path, len, buf) => {
        const parts = w.resolve(fd, w.str(path, len));
        const node = parts && w.files.lookup(parts);
        if (!node) return E.NOENT;
        w.stat(node, buf);
        return E.SUCCESS;
      },
      path_filestat_set_times: () => E.SUCCESS,
      path_create_directory: (fd, path, len) => {
        const parts = w.resolve(fd, w.str(path, len));
        if (!parts) return E.BADF;
        if (w.files.lookup(parts)) return E.EXIST;
        if (parts[0] !== 'data') return E.ACCES;
        const parent = w.files.parent(parts, false);
        if (!parent) return E.NOENT;
        parent.children.set(parts[parts.length - 1], new Dir());
        return E.SUCCESS;
      },
      path_remove_directory: (fd, path, len) => {
        const parts = w.resolve(fd, w.str(path, len));
        const node = parts && w.files.lookup(parts);
        if (!node) return E.NOENT;
        if (!(node instanceof Dir)) return E.NOTDIR;
        if (node.children.size) return E.NOTEMPTY;
        w.files.parent(parts, false).children.delete(parts[parts.length - 1]);
        return E.SUCCESS;
      },
      path_unlink_file: (fd, path, len) => {
        const parts = w.resolve(fd, w.str(path, len));
        const node = parts && w.files.lookup(parts);
        if (!node) return E.NOENT;
        if (node instanceof Dir) return E.ISDIR;
        if (!node.persist) return E.ACCES;
        w.files.parent(parts, false).children.delete(parts[parts.length - 1]);
        w.files.changed();
        return E.SUCCESS;
      },
      path_rename: (fd, old, oldLen, newFd, path, len) => {
        const from = w.resolve(fd, w.str(old, oldLen));
        const to = w.resolve(newFd, w.str(path, len));
        const node = from && w.files.lookup(from);
        if (!node) return E.NOENT;
        if (from[0] !== 'data' || to[0] !== 'data') return E.ACCES;
        const parent = w.files.parent(to, false);
        if (!parent) return E.NOENT;
        w.files.parent(from, false).children.delete(from[from.length - 1]);
        parent.children.set(to[to.length - 1], node);
        w.files.changed();
        return E.SUCCESS;
      },
      path_readlink: () => E.INVAL,
      path_symlink: () => E.NOSYS,
      path_link: () => E.NOSYS,
      poll_oneoff: (input, output, n, nevents) => {
        // the page's thread never sleeps: every wait is over at once
        const v = w.view;
        for (let k = 0; k < n; k++) {
          v.setBigUint64(output + 32 * k, v.getBigUint64(input + 48 * k, true), true);
          v.setUint16(output + 32 * k + 8, 0, true);
          v.setUint8(output + 32 * k + 10, v.getUint8(input + 48 * k + 8));
        }
        v.setUint32(nevents, n, true);
        return E.SUCCESS;
      },
      proc_exit: (code) => {
        throw new Error(`the player stopped (exit ${code})`);
      },
      proc_raise: () => E.NOSYS,
      sched_yield: () => E.SUCCESS,
      sock_accept: () => E.NOSYS,
      sock_recv: () => E.NOSYS,
      sock_send: () => E.NOSYS,
      sock_shutdown: () => E.NOSYS,
    };
  }
}
Wasi.ino = 1;

// ---- keys: the page's KeyboardEvent.code as SDL's scancode (USB's numbers) ----

const SCANCODES = (() => {
  const m = {};
  for (let k = 0; k < 26; k++) m[`Key${String.fromCharCode(65 + k)}`] = 4 + k;
  for (let k = 1; k <= 9; k++) m[`Digit${k}`] = 29 + k;
  m.Digit0 = 39;
  Object.assign(m, {
    Enter: 40, Escape: 41, Backspace: 42, Tab: 43, Space: 44, Minus: 45, Equal: 46, BracketLeft: 47, BracketRight: 48,
    Backslash: 49, Semicolon: 51, Quote: 52, Backquote: 53, Comma: 54, Period: 55, Slash: 56, CapsLock: 57,
    PrintScreen: 70, ScrollLock: 71, Pause: 72, Insert: 73, Home: 74, PageUp: 75, Delete: 76, End: 77, PageDown: 78,
    ArrowRight: 79, ArrowLeft: 80, ArrowDown: 81, ArrowUp: 82, NumLock: 83, NumpadDivide: 84, NumpadMultiply: 85,
    NumpadSubtract: 86, NumpadAdd: 87, NumpadEnter: 88, NumpadDecimal: 99, IntlBackslash: 100, ContextMenu: 101,
    ControlLeft: 224, ShiftLeft: 225, AltLeft: 226, MetaLeft: 227, ControlRight: 228, ShiftRight: 229, AltRight: 230, MetaRight: 231,
  });
  for (let k = 1; k <= 12; k++) m[`F${k}`] = 57 + k;
  for (let k = 1; k <= 9; k++) m[`Numpad${k}`] = 88 + k;
  m.Numpad0 = 98;
  return m;
})();

// Windows' keys for the table: Caps Lock and Enter flip, either Shift plunges. Windows loses the key-up of a Shift
// let go while the other is held (see `key` below), and asks about Sticky Keys after five Shift presses in a row;
// Caps Lock and Enter sit at the keyboard's edges as the Shifts do. The player hears each as the key it stands for
// (the Shifts flip there, Enter plunges: its settings, its menu and the table's script as on the desktop) and is told
// their names (`VPI_KEY_NAMES`, for the pause menu's Controls page)
const WINDOWS = /^Win/.test(navigator.userAgentData?.platform ?? navigator.platform ?? '');
const LAYOUT = WINDOWS ? { CapsLock: SCANCODES.ShiftLeft, Enter: SCANCODES.ShiftRight, ShiftLeft: SCANCODES.Enter, ShiftRight: SCANCODES.Enter } : {};
const LAYOUT_NAMES = WINDOWS ? `${SCANCODES.ShiftLeft}=CapsLock;${SCANCODES.ShiftRight}=Return;${SCANCODES.Enter}=Shift` : '';

// ---- sound: Web Audio, fed each frame from the player's mixer ----

const WORKLET = `
class Feed extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = [];
    this.at = 0;
    this.port.onmessage = (e) => this.chunks.push(e.data);
  }
  process(inputs, outputs) {
    const l = outputs[0][0], r = outputs[0][1] || l;
    let i = 0;
    while (i < l.length && this.chunks.length) {
      const c = this.chunks[0];
      const n = Math.min(l.length - i, (c.length >> 1) - this.at);
      for (let k = 0; k < n; k++) {
        l[i + k] = c[(this.at + k) * 2];
        r[i + k] = c[(this.at + k) * 2 + 1];
      }
      i += n;
      this.at += n;
      if (this.at >= c.length >> 1) {
        this.chunks.shift();
        this.at = 0;
      }
    }
    if (i < l.length) {
      l.fill(0, i);
      r.fill(0, i);
    }
    return true;
  }
}
registerProcessor('vpi-feed', Feed);`;

class Sound {
  constructor() {
    this.ctx = new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' });
    this.node = null;
    this.sent = 0;
    this.start = 0;
    this.ahead = 0.05; // seconds queued ahead of what plays
    this.late = 0; // times the page fell behind and sound ran out
    this.mixMs = []; // what each frame's mixing took the page, lately
  }
  // what mixing takes each frame (the sounds decoded as they play among it), and how often sound ran out
  summary() {
    const t = [...this.mixMs].sort((a, b) => a - b);
    const at = (q) => t[Math.min(t.length - 1, Math.floor(q * t.length))]?.toFixed(2);
    return { state: this.ctx.state, late: this.late, aheadMs: Math.round(this.ahead * 1000), mix_median_ms: at(0.5), mix_p99_ms: at(0.99), mix_max_ms: at(1), baseLatency: this.ctx.baseLatency, outputLatency: this.ctx.outputLatency };
  }
  async ready() {
    const url = URL.createObjectURL(new Blob([WORKLET], { type: 'text/javascript' }));
    await this.ctx.audioWorklet.addModule(url);
    this.node = new AudioWorkletNode(this.ctx, 'vpi-feed', { numberOfInputs: 0, outputChannelCount: [2] });
    this.node.connect(this.ctx.destination);
  }
  // what the mixer has for the time to come, short of `ahead` queued
  pump(exports, memory) {
    if (!this.node || this.ctx.state !== 'running') return;
    if (!this.start) this.start = this.ctx.currentTime;
    const played = (this.ctx.currentTime - this.start) * 48000;
    if (this.sent < played) {
      // the page was too slow: what's late is lost, and more is kept ahead from now on
      this.sent = played;
      this.ahead = Math.min(0.15, this.ahead + 0.01);
      this.late++;
    }
    const want = Math.ceil((played + this.ahead * 48000 - this.sent) / 128) * 128;
    if (want <= 0) return;
    const began = performance.now();
    const p = exports.vpi_web_audio(want);
    this.mixMs.push(performance.now() - began);
    if (this.mixMs.length > 3600) this.mixMs.shift();
    const chunk = new Float32Array(memory.buffer, p, want * 2).slice();
    this.node.port.postMessage(chunk, [chunk.buffer]);
    this.sent += want;
  }
}

// a response's bytes, gunzipped when they're gzip's (package.sh gzips what that shrinks; a host may have
// unzipped them already, or kept a copy in the browser's cache as they came)
async function unzipped(res) {
  const reader = res.body.getReader();
  const first = await reader.read();
  const body = new ReadableStream({
    start(c) {
      if (first.done) c.close();
      else c.enqueue(first.value);
    },
    async pull(c) {
      const { done, value } = await reader.read();
      if (done) c.close();
      else c.enqueue(value);
    },
  });
  const gzip = !first.done && first.value.length > 1 && first.value[0] === 0x1f && first.value[1] === 0x8b;
  return gzip ? body.pipeThrough(new DecompressionStream('gzip')) : body;
}

// ---- the page ----

async function main() {
  // the host's latest, always: the files it names are checked against it as they come
  const manifest = await (await fetch('manifest.json', { cache: 'no-cache' })).json();
  document.title = manifest.title;
  $('title').textContent = manifest.title;
  if (manifest.byline) $('byline').textContent = manifest.byline;
  if (manifest.source) {
    $('source-link').hidden = false;
    $('source-link').querySelector('a').href = manifest.source;
  }

  // where the last visit got to: a phone's browser out of memory reloads the page without a word
  const lastKey = `vpi:${manifest.id}:last-visit`;
  let lastVisit = null;
  try {
    lastVisit = JSON.parse(localStorage.getItem(lastKey) || 'null');
  } catch {}
  // once the page is leaving, nothing more: the browser may tell it it's hidden after it said goodbye
  let leaving = false;
  const reached = (stage, extra = {}) => {
    if (leaving) return;
    try {
      localStorage.setItem(lastKey, JSON.stringify({ stage, at: new Date().toISOString(), ...extra }));
    } catch {}
  };
  if (lastVisit && lastVisit.stage !== 'closed' && Date.now() - Date.parse(lastVisit.at) < 3600e3) {
    $('notice').hidden = false;
    $('notice').textContent = `The last visit ended ${lastVisit.stage}${lastVisit.memoryMB ? `, the program holding ${lastVisit.memoryMB} MB` : ''}, without closing: the browser may have run out of memory for it.`;
  }

  // what the page has and does, for the report: filled in as it starts
  const live = {};
  const report = () => {
    let st = null;
    try {
      st = live.state?.();
    } catch {}
    const a = live.adapter;
    return {
      build: manifest.build ?? null,
      when: new Date().toISOString(),
      userAgent: navigator.userAgent,
      touch: { device: touchDevice, points: navigator.maxTouchPoints, standalone: navigator.standalone === true || matchMedia('(display-mode: standalone)').matches },
      screen: { width: screen.width, height: screen.height, pixelRatio: devicePixelRatio, window: [innerWidth, innerHeight], canvas: live.canvas ? [live.canvas.width, live.canvas.height] : null, view: live.view ?? null },
      webgpu: !navigator.gpu ? 'none' : {
        adapter: a ? { vendor: a.info?.vendor, architecture: a.info?.architecture, description: a.info?.description } : live.adapterTried ? 'none' : null,
        features: live.device ? [...live.device.features] : [],
        maxTextureDimension2D: a?.limits.maxTextureDimension2D,
        maxBufferSize: a?.limits.maxBufferSize,
        lost: live.lost ?? null,
        errors: live.gpu ? [...live.gpu.errorLog.entries()].map(([message, count]) => ({ count, message })) : [],
      },
      memoryMB: live.x ? Math.round(live.x.memory.buffer.byteLength / 1048576) : null,
      streamSoundsOver: live.stream ?? null,
      fetch: live.fetch ?? null,
      startSeconds: live.startSeconds ?? null,
      state: st,
      frames: live.stats?.summary() ?? null,
      sound: live.sound?.summary() ?? null,
      lastVisit,
      lines: lines.slice(-40),
    };
  };
  const panel = $('panel');
  let panelTimer = 0;
  const showReport = () => ($('report').textContent = JSON.stringify(report(), null, 1));
  const openPanel = () => {
    live.pause?.();
    panel.hidden = false;
    showReport();
    clearInterval(panelTimer);
    panelTimer = setInterval(showReport, 1000);
  };
  $('close').onclick = () => {
    panel.hidden = true;
    clearInterval(panelTimer);
    $('copied').textContent = '';
  };
  $('copy').onclick = async () => {
    const text = JSON.stringify(report(), null, 1);
    try {
      await navigator.clipboard.writeText(text);
      $('copied').textContent = 'Copied: paste it into a message.';
    } catch {
      getSelection().selectAllChildren($('report'));
      $('copied').textContent = 'Selected: copy it from the menu.';
    }
  };
  $('overlay-report').onclick = openPanel;
  $('touch-report').onclick = openPanel;
  if (touchDevice) {
    document.body.classList.add('touch');
    $('keys').hidden = true;
    $('touch-keys').hidden = false;
    if (!document.fullscreenEnabled && !report().touch.standalone && /iPhone|iPod/.test(navigator.userAgent)) $('home-hint').hidden = false;
  }
  if (WINDOWS) {
    $('flip-keys').innerHTML = '<kbd>Caps Lock</kbd> <kbd>Enter</kbd>';
    $('plunge-keys').innerHTML = '<kbd>Shift</kbd>';
  }

  if (!navigator.gpu) {
    status('This browser has no WebGPU: Visual Pinball Infinite needs a recent Chrome, Edge, Safari (iOS 26 on an iPhone or iPad) or Firefox.');
    return;
  }
  live.adapterTried = true;
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
  live.adapter = adapter;
  if (!adapter) {
    // a browser turns WebGPU off for a while after a graphics card error, its own or another page's
    status('WebGPU found no graphics card it can use here. If it played here a moment ago, the browser has switched WebGPU off after a graphics error: quit the browser and open it again.');
    return;
  }
  // `?plain` asks for none of WebGPU's optional features, as a browser without them would have it
  const plain = new URLSearchParams(location.search).has('plain');
  const wanted = plain ? [] : ['clip-distances', 'float32-filterable', 'depth-clip-control'].filter((f) => adapter.features.has(f));
  // `?profile`: the graphics card's own times of each pass (gpu.js)
  if (new URLSearchParams(location.search).has('profile') && adapter.features.has('timestamp-query')) wanted.push('timestamp-query');
  const limits = {};
  for (const k of ['maxTextureDimension2D', 'maxBufferSize', 'maxColorAttachmentBytesPerSample', 'maxSampledTexturesPerShaderStage', 'maxSamplersPerShaderStage', 'maxUniformBufferBindingSize', 'maxUniformBuffersPerShaderStage', 'maxDynamicUniformBuffersPerPipelineLayout', 'maxInterStageShaderVariables', 'maxVertexAttributes', 'maxVertexBuffers']) {
    if (adapter.limits[k] !== undefined) limits[k] = adapter.limits[k];
  }
  const device = await adapter.requestDevice({ requiredFeatures: wanted, requiredLimits: limits });
  live.device = device;
  let lost = false;
  const pageStarted = performance.now();
  device.lost.then((info) => {
    lost = true;
    if (info.reason === 'destroyed') return;
    live.lost = `${info.reason}: ${info.message}`;
    console.error(`WebGPU device lost (${info.reason}): ${info.message}`);
    // lost while starting (macOS's GPU recovering from the page before this one, as a reload can
    // make it): one more try on its own
    if (performance.now() - pageStarted < 30000 && !sessionStorage.getItem('vpi-retried')) {
      sessionStorage.setItem('vpi-retried', '1');
      location.reload();
      return;
    }
    $('overlay').hidden = false;
    $('play').hidden = false;
    $('play').textContent = 'Reload';
    $('play').onclick = () => location.reload();
    status('The graphics card stopped the table (the browser’s console says why).');
  });
  const canvas = $('table');
  const context = canvas.getContext('webgpu');
  const format = navigator.gpu.getPreferredCanvasFormat();
  // read back too, for vpi.capture
  context.configure({ device, format, alphaMode: 'opaque', viewFormats: [`${format}-srgb`], usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
  const gpu = new Gpu(device, canvas, context, format, log);
  live.gpu = gpu;
  live.canvas = canvas;

  // the canvas at the screen's pixels (twice the page's at most, one and a half on a phone or tablet:
  // `?dpr=` to choose), as its box changes; the player starts once it has its size, so its first
  // pictures are drawn at it
  const maxDpr = +(params.get('dpr') ?? (touchDevice ? 1.5 : 2)) || 2;
  const maxSide = Math.min(device.limits.maxTextureDimension2D, 3840);
  let resized = null;
  let sized;
  const firstSize = new Promise((r) => (sized = r));
  const fit = (w, h) => {
    const scale = Math.min(1, maxSide / Math.max(w, h));
    canvas.width = Math.max(1, Math.round(w * scale));
    canvas.height = Math.max(1, Math.round(h * scale));
    resized?.(canvas.width, canvas.height);
    sized();
  };
  // `?size=1280x720`: the canvas at that many pixels, whatever its box (pictures to compare with the player's)
  const fixed = new URLSearchParams(location.search).get('size')?.match(/^(\d+)x(\d+)$/);
  if (fixed) {
    canvas.width = +fixed[1];
    canvas.height = +fixed[2];
    canvas.style.objectFit = 'contain';
    sized();
  }
  if (!fixed) new ResizeObserver((entries) => {
    const e = entries[0];
    const box = e.devicePixelContentBoxSize?.[0];
    const dpr = Math.min(window.devicePixelRatio || 1, maxDpr);
    if (box && dpr === (window.devicePixelRatio || 1)) fit(box.inlineSize, box.blockSize);
    else fit(e.contentRect.width * dpr, e.contentRect.height * dpr);
  }).observe(canvas);

  // the table's files, fetched with their progress: a big one in its pieces (package.sh cuts them, for
  // hosts that cap a file's size), three at a time; each kept in the browser's cache by its contents,
  // so the next visit has them at once and a new version fetches what changed
  reached('while fetching the table');
  const files = new Files(`vpi:${manifest.id}:data`);
  files.mkdir('/data/table-settings');
  files.load();
  const total = manifest.files.reduce((n, f) => n + f.size, 0);
  let got = 0, fromCache = 0, shown = 0;
  const bar = $('progress');
  const cache = await globalThis.caches?.open(`vpi:${manifest.id}`).catch(() => null);
  const kept = new Set();
  const fetchStarted = performance.now();
  // whether the bytes read are those the manifest names (its hash of them), where the browser can hash
  const named = async (into, at, size, content) => {
    if (!content || !globalThis.crypto?.subtle) return true;
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', into.subarray(at, at + size)));
    return [...hash.subarray(0, 8)].map((b) => b.toString(16).padStart(2, '0')).join('') === content;
  };
  // reads `res` into `into` at `at`, counted in the progress: how many bytes, or -1 (uncounted) when
  // it's longer than `size`
  const fill = async (res, size, into, at) => {
    const reader = (await unzipped(res)).getReader();
    let n = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (n + value.length > size) {
        reader.cancel().catch(() => {});
        got -= n;
        return -1;
      }
      into.set(value, at + n);
      n += value.length;
      got += value.length;
      bar.value = got / total;
      if (performance.now() - shown > 200) {
        shown = performance.now();
        status(`Fetching the table: ${Math.round(got / 1e6)} of ${Math.round(total / 1e6)} MB…`);
      }
    }
    return n;
  };
  const piece = async (url, size, sha, content, into, at) => {
    const key = sha ? new URL(`${url}?v=${sha}`, location.href).href : null;
    if (key) kept.add(key);
    // kept from an earlier visit, when it's whole and what the manifest says
    const keptCopy = key && cache ? await cache.match(key).catch(() => null) : null;
    if (keptCopy) {
      const n = await fill(keptCopy, size, into, at);
      if (n === size && (await named(into, at, size, content))) {
        fromCache += size;
        return;
      }
      if (n > 0) got -= n;
      cache.delete(key).catch(() => {});
    }
    // else from the host, by this version's own address; and once more, past every cache, when what
    // came isn't what the manifest says (a cache's older copy, the site having just changed: GitHub
    // Pages' lets browsers keep a file ten minutes, and its own cache goes by the address without `?v`)
    for (let attempt = 0; ; attempt++) {
      const res = await fetch(key ?? url, attempt ? { cache: 'reload' } : {});
      if (!res.ok) throw new Error(`${url}: ${res.status}`);
      const n = await fill(res, size, into, at);
      if (n === size && (await named(into, at, size, content))) break;
      if (n > 0) got -= n;
      if (attempt) throw new Error(`${url} isn't what the manifest says: the site may have just changed. Reload the page in a minute.`);
      await new Promise((r) => setTimeout(r, 1500));
    }
    if (key && cache) await cache.put(key, new Response(into.slice(at, at + size))).catch(() => {});
  };
  // `jobs`, `n` at a time
  const inTurn = async (jobs, n) => {
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(n, jobs.length) }, async () => {
      while (next < jobs.length) await jobs[next++]();
    }));
  };
  const fetchFile = async (f) => {
    const data = new Uint8Array(f.size);
    if (f.parts) {
      let at = 0;
      await inTurn(f.parts.map((p) => {
        const from = at;
        at += p.size;
        return () => piece(p.url, p.size, p.sha256, p.content, data, from);
      }), 3);
    } else await piece(f.url, f.size, f.sha256, f.content, data, 0);
    files.put(f.path, data, false);
  };
  // the small files eight at a time, the big ones (the table) after, one by one
  const big = (f) => f.parts || f.size >= 4 << 20;
  await inTurn(manifest.files.filter((f) => !big(f)).map((f) => () => fetchFile(f)), 8);
  for (const f of manifest.files.filter(big)) await fetchFile(f);
  // what an earlier version left in the cache
  if (cache) for (const req of await cache.keys().catch(() => [])) if (!kept.has(req.url)) cache.delete(req).catch(() => {});
  live.fetch = { seconds: +((performance.now() - fetchStarted) / 1000).toFixed(1), totalMB: Math.round(total / 1e6), fromCacheMB: Math.round(fromCache / 1e6) };
  reached('while starting the player');
  status('Starting the player…');
  // compiled as it arrives; whole, where the host doesn't serve it as application/wasm
  const wasmUrl = manifest.program ? `${manifest.program.url}?v=${manifest.program.sha256}` : 'vpi.wasm';
  const program = async () => new Response(await unzipped(await fetch(wasmUrl)), { headers: { 'Content-Type': 'application/wasm' } });
  const wasm = await WebAssembly.compileStreaming(program()).catch(async () => WebAssembly.compile(await (await program()).arrayBuffer()));
  // the table's view: the settings' (the desktop view, which held upright fills a phone's height), or
  // `?view=fss` (the table's full single screen view) or `?view=desktop`
  const view = params.get('view');
  live.view = view ?? 'settings';
  const env = { VPI_TABLE: manifest.table, VPI_DATA: '/data', VPI_TABLE_SETTINGS: '/data/table-settings', HOME: '/data' };
  if (view) env.VPI_VIEW = view;
  if (LAYOUT_NAMES) env.VPI_KEY_NAMES = LAYOUT_NAMES;
  // `?pictures=program`: the table's pictures decoded by the program, not the browser
  if (params.get('pictures') === 'program') env.VPI_PICTURES = 'program';
  // sounds longer than this many seconds kept as their files and decoded as they play (music, mostly), the
  // shorter ones decoded as the table loads: `?stream=0` decodes them all at once
  env.VPI_STREAM_SOUNDS = params.get('stream') ?? String(manifest.streamSounds ?? 0);
  live.stream = env.VPI_STREAM_SOUNDS;
  const wasi = new Wasi(files, env, (fd, line) => {
    keep(line);
    if (fd === 2) console.warn(line);
    else console.log(line);
  });
  const sound = new Sound();
  live.sound = sound;
  const page = {
    page_pixel_density: () => canvas.width / Math.max(1, canvas.clientWidth),
    page_screen_size: (wp, hp) => {
      const v = new Uint32Array(memory().buffer);
      const d = window.devicePixelRatio || 1;
      v[wp >> 2] = Math.round(screen.width * d);
      v[hp >> 2] = Math.round(screen.height * d);
    },
    page_refresh_rate: () => refresh,
    page_fullscreen: (on) => (on ? canvas.requestFullscreen?.().catch(() => {}) : document.exitFullscreen?.().catch(() => {})),
    page_rumble: (index, strong, weak, ms) => {
      const pad = navigator.getGamepads()[index];
      pad?.vibrationActuator?.playEffect?.('dual-rumble', { duration: ms, strongMagnitude: strong, weakMagnitude: weak }).catch(() => {});
    },
  };
  const instance = await WebAssembly.instantiate(wasm, { wasi_snapshot_preview1: wasi.imports(), vpi_gpu: gpu.imports(), vpi_page: page });
  const x = instance.exports;
  live.x = x;
  const memory = () => x.memory;
  const memoryMB = () => Math.round(x.memory.buffer.byteLength / 1048576);
  wasi.memory = x.memory;
  gpu.memory = x.memory;
  x.vpi_web_init();
  await sound.ready();

  // the screen's refresh, from the frames' times
  let refresh = 60;
  const times = [];
  status('Loading the table… (a big table takes a while)');
  await firstSize;
  await new Promise((r) => setTimeout(r, 30));
  reached('while loading the table', { memoryMB: memoryMB() });
  const t0 = performance.now();
  if (!x.vpi_web_start(refresh)) {
    status('The table could not start: its Test report says why.');
    return;
  }
  // its pictures, decoded by the browser as the table asked for them
  status('Decoding the pictures…');
  await gpu.pictures();
  // the graphics card done with the table's start (its pictures uploaded, its environment's light
  // worked out) before any frame adds to it
  await device.queue.onSubmittedWorkDone();
  live.startSeconds = +((performance.now() - t0) / 1000).toFixed(1);
  log(`the table started in ${live.startSeconds} s`);
  reached('waiting for Play', { memoryMB: memoryMB() });
  setTimeout(() => sessionStorage.removeItem('vpi-retried'), 30000);
  resized = (w, h) => x.vpi_web_resized(w, h);

  // play waits for a click (the browser plays sound only after one)
  let playing = false;
  x.vpi_web_focus(0);
  const overlay = $('overlay');
  status('');
  $('play').hidden = false;

  // touches: each holds a key down (a flipper, or a button of the bar) until it lifts
  const holding = new Map(); // a key's scancode: how many touches hold it
  const touches = new Map(); // a touch's pointerId: what it does
  const pressed = new Set(); // the keyboard's keys the player has down
  const hold = (code, down) => {
    const was = holding.get(code) ?? 0;
    const n = Math.max(0, was + (down ? 1 : -1));
    holding.set(code, n);
    if (!was && n) x.vpi_web_key(code, 1, 0);
    if (was && !n) x.vpi_web_key(code, 0, 0);
  };
  // every touch and key let go: a touch the browser takes over never lifts, and a key let go while the
  // page hasn't the keyboard never comes up (SDL lets the keys go as its window loses the keyboard)
  const letGo = () => {
    for (const [code, n] of holding) if (n) x.vpi_web_key(code, 0, 0);
    for (const code of pressed) x.vpi_web_key(code, 0, 0);
    holding.clear();
    touches.clear();
    pressed.clear();
    document.querySelectorAll('#touch .down').forEach((p) => p.classList.remove('down'));
  };
  let hinted = false;
  const go = async () => {
    overlay.hidden = true;
    canvas.focus();
    playing = true;
    x.vpi_web_focus(1);
    document.body.classList.add('playing');
    if (touchDevice) {
      $('touch').hidden = false;
      if (!hinted) {
        hinted = true;
        setTimeout(() => document.querySelectorAll('#touch .hint').forEach((h) => h.classList.add('gone')), 6000);
      }
    }
    // the browser lets sound start only after a press of its own choosing (a gamepad's may not
    // count, and a phone's only when the finger lifts): until it does, the next click, key or touch starts it
    await sound.ctx.resume().catch(() => {});
    if (sound.ctx.state !== 'running') {
      const kinds = ['pointerdown', 'pointerup', 'touchend', 'keydown'];
      const unlock = () => sound.ctx.resume().then(() => {
        if (sound.ctx.state === 'running') kinds.forEach((k) => window.removeEventListener(k, unlock, true));
      });
      kinds.forEach((k) => window.addEventListener(k, unlock, true));
    }
  };
  const pause = () => {
    if (!playing) return;
    playing = false;
    letGo();
    x.vpi_web_focus(0);
  };
  live.pause = pause;
  $('play').onclick = go;
  canvas.addEventListener('pointerdown', () => {
    if (!playing && overlay.hidden && panel.hidden) go();
  });
  window.addEventListener('blur', pause);
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) return;
    pause();
    reached('in the background', { memoryMB: memoryMB() });
  });
  window.addEventListener('pagehide', () => {
    reached('closed');
    leaving = true;
    files.save();
    // the graphics card's memory let go at once, not when the browser gets round to it
    device.destroy();
  });

  // keys and mouse. A key reaches the player as SDL hands keys on: pressed while it's down, a repeat
  // (whatever the browser says); let go while it's up, nothing. Windows sends no key-up for the Shift let
  // go first while both are held, and the one it sends as the other lifts is the other's: there, as GLFW
  // does, a Shift let go lets both go (what the one let go first holds lets go only as the other does)
  const shifts = ['ShiftLeft', 'ShiftRight'];
  const key = (e, down) => {
    // the key the player hears for it: Windows' own keys (`LAYOUT`) as the ones they stand for
    const code = LAYOUT[e.code] ?? SCANCODES[e.code];
    // a press with Ctrl or ⌘ held is the browser's (a shortcut), a key let go the player's still
    if (code === undefined || (down && (e.ctrlKey || e.metaKey))) return;
    if (!playing) {
      if (down && overlay.hidden) go();
      return;
    }
    e.preventDefault();
    if (down) {
      x.vpi_web_key(code, 1, pressed.has(code) ? 1 : 0);
      pressed.add(code);
    } else if (pressed.delete(code)) {
      x.vpi_web_key(code, 0, 0);
      if (WINDOWS && shifts.includes(e.code)) {
        for (const s of shifts) {
          const c = LAYOUT[s] ?? SCANCODES[s];
          if (pressed.delete(c)) x.vpi_web_key(c, 0, 0);
        }
      }
    }
  };
  window.addEventListener('keydown', (e) => key(e, true));
  window.addEventListener('keyup', (e) => key(e, false));
  const at = (e) => {
    const r = canvas.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };
  canvas.addEventListener('pointermove', (e) => e.pointerType !== 'touch' && x.vpi_web_mouse_move(...at(e)));
  canvas.addEventListener('pointerdown', (e) => playing && e.pointerType !== 'touch' && x.vpi_web_mouse_button([1, 2, 3, 4, 5][e.button] ?? 1, 1, ...at(e)));
  canvas.addEventListener('pointerup', (e) => playing && e.pointerType !== 'touch' && x.vpi_web_mouse_button([1, 2, 3, 4, 5][e.button] ?? 1, 0, ...at(e)));
  canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    x.vpi_web_mouse_wheel(-Math.sign(e.deltaY));
  }, { passive: false });

  // touch on the table: the left half holds the left flipper, the right half the right; with the pause
  // menu up, the menu's mouse instead: a tap chooses, a drag up or down scrolls, a drag sideways moves a bar
  const menuShows = () => {
    try {
      return state().menu;
    } catch {
      return false;
    }
  };
  canvas.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch' || !playing) return;
    canvas.setPointerCapture(e.pointerId);
    const [px, py] = at(e);
    if (menuShows()) {
      x.vpi_web_mouse_move(px, py);
      touches.set(e.pointerId, { menu: true, x0: px, y0: py, y: py, mode: null });
    } else {
      const code = px < canvas.clientWidth / 2 ? SCANCODES.ShiftLeft : SCANCODES.ShiftRight;
      touches.set(e.pointerId, { code });
      hold(code, true);
    }
  });
  canvas.addEventListener('pointermove', (e) => {
    const t = touches.get(e.pointerId);
    if (!t?.menu) return;
    const [px, py] = at(e);
    if (!t.mode && Math.hypot(px - t.x0, py - t.y0) > 12) {
      t.mode = Math.abs(px - t.x0) > Math.abs(py - t.y0) ? 'drag' : 'scroll';
      if (t.mode === 'drag') x.vpi_web_mouse_button(1, 1, t.x0, t.y0);
    }
    if (t.mode === 'drag') x.vpi_web_mouse_move(px, py);
    // a finger moving up brings what's below into view, as a wheel turned down does
    while (t.mode === 'scroll' && Math.abs(py - t.y) >= 40) {
      const dir = Math.sign(py - t.y);
      x.vpi_web_mouse_wheel(dir);
      t.y += dir * 40;
    }
  });
  const lift = (e) => {
    const t = touches.get(e.pointerId);
    if (!t) return;
    touches.delete(e.pointerId);
    if (t.code !== undefined) hold(t.code, false);
    else if (t.mode === 'drag') x.vpi_web_mouse_button(1, 0, ...at(e));
    else if (!t.mode && e.type === 'pointerup') {
      x.vpi_web_mouse_button(1, 1, t.x0, t.y0);
      x.vpi_web_mouse_button(1, 0, t.x0, t.y0);
    }
  };
  canvas.addEventListener('pointerup', lift);
  canvas.addEventListener('pointercancel', lift);
  // the bar's buttons and Launch: their keys, held as long as the touch
  for (const pad of document.querySelectorAll('#touch .pad[data-key]')) {
    const code = SCANCODES[pad.dataset.key];
    const up = () => {
      if (!pad.classList.contains('down')) return;
      pad.classList.remove('down');
      hold(code, false);
    };
    pad.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      pad.setPointerCapture(e.pointerId);
      if (!playing) go();
      pad.classList.add('down');
      hold(code, true);
    });
    pad.addEventListener('pointerup', up);
    pad.addEventListener('pointercancel', up);
  }

  const fullscreen = async () => {
    if (document.fullscreenElement) await document.exitFullscreen();
    else {
      await document.documentElement.requestFullscreen();
      // Esc reaches the table (the pause menu); holding it leaves full screen
      await navigator.keyboard?.lock?.(['Escape']).catch(() => {});
    }
    canvas.focus();
  };
  $('fullscreen').onclick = fullscreen;
  // an iPad has full screen, an iPhone doesn't (its home screen does instead)
  $('touch-full').hidden = !document.fullscreenEnabled;
  $('touch-full').onclick = fullscreen;

  let ended = false;
  // gamepads: each frame, every pad's state to the player
  const encoder = new TextEncoder();
  const known = new Set();
  const pads = () => {
    const now = new Set();
    for (const pad of navigator.getGamepads()) {
      if (!pad || !pad.connected) continue;
      now.add(pad.index);
      const name = encoder.encode(pad.id);
      const nameEnd = (name.length + 3) & ~3;
      const nb = pad.buttons.length, na = pad.axes.length;
      const p = x.vpi_web_scratch(nameEnd + 4 * (nb + na));
      new Uint8Array(x.memory.buffer, p, name.length).set(name);
      const fl = new Float32Array(x.memory.buffer, p + nameEnd, nb + na);
      pad.buttons.forEach((b, k) => (fl[k] = b.value || (b.pressed ? 1 : 0)));
      pad.axes.forEach((a, k) => (fl[nb + k] = a));
      x.vpi_web_pad(pad.index, pad.mapping === 'standard' ? 1 : 0, name.length, nb, na);
      // a press while play waits starts it again, as a click or a key does
      if (!playing && !ended && document.hasFocus() && $('play').textContent === 'Play' && pad.buttons.some((b) => b.pressed)) go();
    }
    for (const k of known) if (!now.has(k)) x.vpi_web_pad_gone(k);
    known.clear();
    now.forEach((k) => known.add(k));
  };

  // what a frame takes the page, for the console (vpi.stats)
  const stats = {
    times: [],
    add(ms) {
      this.times.push(ms);
      if (this.times.length > 240) this.times.shift();
    },
    summary() {
      const t = [...this.times].sort((a, b) => a - b);
      const at = (q) => t[Math.min(t.length - 1, Math.floor(q * t.length))]?.toFixed(2);
      return { frames: t.length, median_ms: at(0.5), p90_ms: at(0.9), max_ms: at(1), refresh_hz: refresh.toFixed(1) };
    },
  };
  live.stats = stats;
  const state = () => {
    const n = x.vpi_web_state();
    return JSON.parse(new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.vpi_web_scratch(0), n)));
  };
  live.state = state;
  // the frame drawn now, as the canvas holds it, saved by the page's server (tools/web/serve.py --captures)
  const capture = async (name) => {
    x.vpi_web_frame();
    const tex = gpu.current ?? context.getCurrentTexture();
    const [w, h] = [tex.width, tex.height];
    const row = Math.ceil((w * 4) / 256) * 256;
    const buffer = device.createBuffer({ size: row * h, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
    const encoder = device.createCommandEncoder();
    encoder.copyTextureToBuffer({ texture: tex }, { buffer, bytesPerRow: row }, { width: w, height: h });
    device.queue.submit([encoder.finish()]);
    gpu.presented();
    await buffer.mapAsync(GPUMapMode.READ);
    const src = new Uint8Array(buffer.getMappedRange());
    const rgba = new Uint8ClampedArray(w * h * 4);
    const bgra = format.startsWith('bgra');
    for (let y = 0; y < h; y++) {
      for (let k = 0; k < w; k++) {
        const s = y * row + k * 4, d = (y * w + k) * 4;
        rgba[d] = src[s + (bgra ? 2 : 0)];
        rgba[d + 1] = src[s + 1];
        rgba[d + 2] = src[s + (bgra ? 0 : 2)];
        rgba[d + 3] = 255;
      }
    }
    buffer.unmap();
    buffer.destroy();
    const off = new OffscreenCanvas(w, h);
    off.getContext('2d').putImageData(new ImageData(rgba, w, h), 0, 0);
    const png = await off.convertToBlob({ type: 'image/png' });
    const res = await fetch(`capture?name=${encodeURIComponent(name)}`, { method: 'POST', body: png });
    return { name, width: w, height: h, saved: res.ok };
  };
  window.vpi = { exports: x, gpu, sound, files, stats, device, state, capture, report };

  // the frames
  let last = 0, lastReached = 0;
  const frame = (t) => {
    if (ended || lost) return;
    if (last) {
      times.push(t - last);
      if (times.length === 120) {
        const sorted = [...times].sort((a, b) => a - b);
        const hz = 1000 / sorted[60];
        if (Math.abs(hz - refresh) > 1) {
          refresh = hz;
          x.vpi_web_refresh(hz);
        }
        times.length = 0;
      }
    }
    last = t;
    requestAnimationFrame(frame);
    const began = performance.now();
    let r;
    try {
      pads();
      r = x.vpi_web_frame();
    } catch (e) {
      // the program stopped (a Rust panic aborts it): said, and the page stops with it
      console.error(e);
      r = -1;
    }
    gpu.presented();
    sound.pump(x, x.memory);
    stats.add(performance.now() - began);
    if (t - lastReached > 5000) {
      lastReached = t;
      reached(playing ? 'while playing' : 'waiting for Play', { memoryMB: memoryMB() });
    }
    if (r <= 0) {
      ended = true;
      reached('closed');
      files.save();
      letGo();
      document.body.classList.remove('playing');
      $('touch').hidden = true;
      overlay.hidden = false;
      $('play').hidden = false;
      $('play').textContent = 'Play again';
      $('play').onclick = () => location.reload();
      status(r === 0 ? 'Thanks for playing.' : 'The table stopped: the browser’s console says why.');
    }
  };
  // `?manual`: no frames of its own, only those vpi.step(n) asks for (to time the graphics card's work)
  const manual = new URLSearchParams(location.search).has('manual');
  window.vpi.step = (n = 1) => {
    for (let k = 0; k < n; k++) {
      x.vpi_web_frame();
      gpu.presented();
    }
    return window.vpi.state();
  };
  if (!manual) requestAnimationFrame(frame);

  // `?autotest=<name>`: plays a few seconds by itself (`&seconds=4`), then saves its picture and a report
  // (the browser, its WebGPU, every WebGPU error) through the page's server (serve.py --captures)
  const autotest = new URLSearchParams(location.search).get('autotest');
  if (autotest) {
    playing = true;
    x.vpi_web_focus(1);
    const seconds = +(new URLSearchParams(location.search).get('seconds') ?? 4);
    await new Promise((r) => setTimeout(r, seconds * 1000));
    const picture = await capture(`${autotest}.png`);
    const result = { ...report(), canvasFormat: format, profile: gpu.profiling ? gpu.profile() : null, picture };
    await fetch(`capture?name=${encodeURIComponent(autotest)}.json`, { method: 'POST', body: JSON.stringify(result, null, 1) });
    // done: no more frames, the graphics card let go
    ended = true;
    device.destroy();
    overlay.hidden = false;
    $('play').hidden = true;
    status(`Test ${autotest} saved: this tab can be closed.`);
  }
}

main().catch((e) => {
  status(`Something went wrong: ${e.message}`);
  console.error(e);
});
