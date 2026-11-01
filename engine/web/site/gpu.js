// The page's side of the player's GPU (engine/web/src/sdl/gpu.rs): SDL's GPU calls on WebGPU.
// Resources are made here when the program asks and known to it by number; a command buffer comes
// as words, played here into one or more of WebGPU's command encoders when it's submitted.

const SWAPCHAIN = 0xFFFFFFFF;

// what each texture format is, for bindings: depth, and whether it filters
const DEPTH = new Set(['depth16unorm', 'depth24plus', 'depth32float', 'depth24plus-stencil8', 'depth32float-stencil8']);
const FLOAT32 = new Set(['r32float', 'rg32float', 'rgba32float']);
const INTEGER = new Set(['r8uint', 'r8sint', 'r16uint', 'r16sint', 'r32uint', 'r32sint', 'rgba8uint', 'rgba8sint']);

// a shader's header from build.rs: what each binding holds, and the colours it writes
function parseHeader(code) {
  const info = { textures: [], samplers: [], uniforms: [], outputs: [] };
  for (const line of code.split('\n', 64)) {
    if (!line.startsWith('// vpi ')) {
      if (line.startsWith('diagnostic') || line === '') break;
      continue;
    }
    const w = line.slice(7).split(' ');
    if (w[0] === 'texture') info.textures.push({ group: +w[1], binding: +w[2], dimension: w[3], kind: w[4], multisampled: w[5] === '1' });
    else if (w[0] === 'sampler') info.samplers.push({ group: +w[1], binding: +w[2], comparison: w[3] === '1' });
    else if (w[0] === 'uniform') info.uniforms.push({ group: +w[1], binding: +w[2], size: +w[3] });
    else if (w[0] === 'output') info.outputs.push(+w[1]);
  }
  return info;
}

export class Gpu {
  constructor(device, canvas, context, format, log) {
    this.device = device;
    this.canvas = canvas;
    this.context = context;
    this.format = format; // the canvas's: bgra8unorm or rgba8unorm
    this.log = log;
    this.srgb = false;
    this.memory = null;
    this.float32Filterable = device.features.has('float32-filterable');
    this.unclippedDepth = device.features.has('depth-clip-control');
    this.clipDistances = device.features.has('clip-distances');
    this.textures = [null];
    this.buffers = [null];
    this.samplers = [null];
    this.shaders = [null];
    this.pipelines = [null];
    this.free = { 0: [], 1: [], 2: [], 3: [], 4: [] };
    this.bindGroups = new Map();
    this.layouts = new Map();
    this.uniformBuffer = null;
    this.uniformSize = 0;
    this.uniformGroups = new Map();
    this.mipPipelines = new Map();
    this.mipSampler = device.createSampler({ minFilter: 'linear', magFilter: 'linear' });
    this.nearest = new Map(); // a sampler's non-filtering twin
    this.emptyLayout = device.createBindGroupLayout({ entries: [] });
    this.emptyGroup = device.createBindGroup({ layout: this.emptyLayout, entries: [] });
    this.decoder = new TextDecoder();
    this.errors = 0;
    this.errorLog = new Map(); // each distinct message, and how often
    device.addEventListener('uncapturederror', (e) => {
      const m = e.error.message;
      this.errorLog.set(m, (this.errorLog.get(m) ?? 0) + 1);
      if (this.errors++ < 20) this.log(`WebGPU: ${m}`);
    });
    this.width = 0;
    this.height = 0;
    this.current = null; // this frame's canvas texture
  }

  // the program's imports (module vpi_gpu)
  imports() {
    return {
      gpu_create_texture: (p, n) => this.createTexture(JSON.parse(this.text(p, n))),
      gpu_create_buffer: (size, usage) => this.createBuffer(size, usage),
      gpu_create_sampler: (p, n) => this.createSampler(JSON.parse(this.text(p, n))),
      gpu_create_shader: (p, n, e, en) => this.createShader(this.text(p, n), this.text(e, en)),
      gpu_create_pipeline: (p, n) => this.createPipeline(JSON.parse(this.text(p, n))),
      gpu_release: (kind, id) => this.release(kind, id),
      gpu_submit: (words, count, uniforms, len) => this.submit(words, count, uniforms, len),
      gpu_canvas_size: (wp, hp) => {
        const v = new Uint32Array(this.memory.buffer);
        v[wp >> 2] = this.canvas.width;
        v[hp >> 2] = this.canvas.height;
      },
      gpu_canvas_format: () => (this.format === 'rgba8unorm' ? 1 : 0),
      gpu_set_canvas_srgb: (on) => {
        this.srgb = on !== 0;
      },
      gpu_has_feature: (f) => [this.clipDistances, this.float32Filterable, this.unclippedDepth][f] ? 1 : 0,
    };
  }

  text(p, n) {
    return this.decoder.decode(new Uint8Array(this.memory.buffer, p, n));
  }

  slot(kind, table, value) {
    const id = this.free[kind].pop() ?? table.length;
    table[id] = value;
    return id;
  }

  release(kind, id) {
    const table = [this.textures, this.buffers, this.samplers, this.shaders, this.pipelines][kind];
    const v = table[id];
    if (!v) return;
    if (kind === 0) v.texture.destroy();
    if (kind === 1) v.destroy();
    table[id] = null;
    // numbers come back in use: the bind groups that held this one go
    if (kind === 0 || kind === 2) this.bindGroups.clear();
    this.free[kind].push(id);
  }

  createTexture(d) {
    const depth = DEPTH.has(d.format);
    // depth copies only where WebGPU has them: whole 16- and 32-bit depth pictures
    let usage = depth && d.format !== 'depth16unorm' && d.format !== 'depth32float' ? 0 : GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC;
    if (d.sampled || d.samples === 1) usage |= GPUTextureUsage.TEXTURE_BINDING;
    if (d.target) usage |= GPUTextureUsage.RENDER_ATTACHMENT;
    // its mipmaps drawn here need it drawn into
    if (d.levels > 1 && !depth && !INTEGER.has(d.format)) usage |= GPUTextureUsage.RENDER_ATTACHMENT;
    if (d.samples > 1) usage &= ~GPUTextureUsage.COPY_SRC & ~GPUTextureUsage.COPY_DST & ~GPUTextureUsage.TEXTURE_BINDING;
    const layers = d.kind === 'cube' ? 6 : d.kind === 'cube-array' ? Math.max(6, d.layers) : d.layers;
    const levels = Math.min(d.levels, Math.floor(Math.log2(Math.max(d.width, d.height, d.kind === '3d' ? d.layers : 1))) + 1);
    try {
      const texture = this.device.createTexture({
        size: { width: d.width, height: d.height, depthOrArrayLayers: layers },
        format: d.format,
        dimension: d.kind === '3d' ? '3d' : '2d',
        mipLevelCount: levels,
        sampleCount: d.samples,
        usage,
      });
      const view = texture.createView({ dimension: d.kind === '3d' ? '3d' : d.kind === 'cube' ? 'cube' : d.kind === 'cube-array' ? 'cube-array' : layers > 1 ? '2d-array' : '2d' });
      const unfilterable = depth || INTEGER.has(d.format) || (FLOAT32.has(d.format) && !this.float32Filterable);
      return this.slot(0, this.textures, { texture, view, format: d.format, levels, layers, kind: d.kind, depth, unfilterable, views: new Map(), serial: ++Gpu.serial });
    } catch (e) {
      this.log(`texture ${JSON.stringify(d)}: ${e.message}`);
      return 0;
    }
  }

  createBuffer(size, usage) {
    let u = GPUBufferUsage.COPY_DST | GPUBufferUsage.COPY_SRC;
    if (usage & 1) u |= GPUBufferUsage.VERTEX;
    if (usage & 2) u |= GPUBufferUsage.INDEX;
    try {
      return this.slot(1, this.buffers, this.device.createBuffer({ size: Math.max(4, size), usage: u }));
    } catch (e) {
      this.log(`buffer ${size}: ${e.message}`);
      return 0;
    }
  }

  createSampler(d) {
    const s = this.device.createSampler(d);
    return this.slot(2, this.samplers, { sampler: s, desc: d, serial: ++Gpu.serial });
  }

  // a sampler that doesn't filter, for a texture that can't be filtered
  nonFiltering(s) {
    let n = this.nearest.get(s.serial);
    if (!n) {
      const d = { ...s.desc, minFilter: 'nearest', magFilter: 'nearest', mipmapFilter: 'nearest', maxAnisotropy: 1 };
      delete d.compare;
      n = this.device.createSampler(d);
      this.nearest.set(s.serial, n);
    }
    return n;
  }

  createShader(code, entry) {
    // a mirror's vertex shader: its twin without clip distances where WebGPU hasn't them
    const [clipped, unclipped] = code.split('\n//// vpi without clip distances\n');
    code = unclipped !== undefined && !this.clipDistances ? unclipped : clipped;
    try {
      const module = this.device.createShaderModule({ code, label: entry });
      return this.slot(3, this.shaders, { module, entry, info: parseHeader(code), code });
    } catch (e) {
      this.log(`shader ${entry}: ${e.message}`);
      return 0;
    }
  }

  // the bind group layouts of a vertex and fragment shader pair: groups 0 and 2 their textures and
  // samplers, 1 and 3 their uniforms; `unfilterable` the fragment textures (by binding) bound now
  // that can't be filtered, and their samplers
  layoutsFor(vs, fs, unfilterable) {
    const key = `${vs.serial}/${fs.serial}/${[...unfilterable].join(',')}`;
    let l = this.layouts.get(key);
    if (l) return l;
    const entries = [[], [], [], []];
    for (const [shader, visibility, groups] of [[vs, GPUShaderStage.VERTEX, [0, 1]], [fs, GPUShaderStage.FRAGMENT, [2, 3]]]) {
      for (const t of shader.info.textures) {
        const loose = visibility === GPUShaderStage.FRAGMENT && unfilterable.has(t.binding);
        const sampleType = t.kind === 'float' ? (loose ? 'unfilterable-float' : 'float') : t.kind;
        entries[t.group].push({ binding: t.binding, visibility, texture: { sampleType, viewDimension: t.dimension, multisampled: t.multisampled } });
      }
      for (const s of shader.info.samplers) {
        const loose = visibility === GPUShaderStage.FRAGMENT && unfilterable.has(s.binding - 1);
        entries[s.group].push({ binding: s.binding, visibility, sampler: { type: s.comparison ? 'comparison' : loose ? 'non-filtering' : 'filtering' } });
      }
      for (const u of shader.info.uniforms) {
        entries[u.group].push({ binding: u.binding, visibility, buffer: { type: 'uniform', hasDynamicOffset: true, minBindingSize: u.size } });
      }
    }
    // in order of binding, as the dynamic offsets go
    for (const e of entries) e.sort((a, b) => a.binding - b.binding);
    const groups = entries.map((e) => (e.length ? this.device.createBindGroupLayout({ entries: e }) : this.emptyLayout));
    l = { groups, layout: this.device.createPipelineLayout({ bindGroupLayouts: groups }), entries, serial: ++Gpu.serial };
    this.layouts.set(key, l);
    return l;
  }

  createPipeline(d) {
    const vs = this.shaders[d.vs];
    const fs = this.shaders[d.fs];
    if (!vs || !fs) return 0;
    vs.serial ??= ++Gpu.serial;
    fs.serial ??= ++Gpu.serial;
    const p = { d, vs, fs, variants: new Map() };
    const id = this.slot(4, this.pipelines, p);
    // the usual variant made now, so its shaders compile while the table loads
    this.variant(p, new Set());
    return id;
  }

  // a pipeline for the fragment textures that can't be filtered among those bound
  variant(p, unfilterable) {
    const key = [...unfilterable].join(',');
    let v = p.variants.get(key);
    if (v) return v;
    const { d, vs, fs } = p;
    const layouts = this.layoutsFor(vs, fs, unfilterable);
    const written = new Set(fs.info.outputs);
    const targets = d.targets.map((t, k) => {
      const target = { format: t.format, writeMask: written.has(k) ? t.mask : 0 };
      if (t.blend && written.has(k)) target.blend = t.blend;
      return target;
    });
    // the vertex buffers at their slots, as SDL numbers them (none where it has none)
    const buffers = [];
    for (const b of d.buffers) buffers[b.slot] = { arrayStride: b.stride, stepMode: b.step, attributes: b.attributes.map((a) => ({ shaderLocation: a.location, offset: a.offset, format: a.format })) };
    for (let k = 0; k < buffers.length; k++) buffers[k] ??= null;
    const desc = {
      layout: layouts.layout,
      vertex: { module: vs.module, entryPoint: vs.entry, buffers },
      fragment: { module: fs.module, entryPoint: fs.entry, targets },
      primitive: { topology: d.topology, cullMode: d.cull, frontFace: d.front },
      multisample: { count: d.samples, alphaToCoverageEnabled: d.alphaToCoverage },
      label: `${vs.entry}+${fs.entry}`,
    };
    if (d.unclipped && this.unclippedDepth) desc.primitive.unclippedDepth = true;
    if (d.depth) desc.depthStencil = { ...d.depth };
    try {
      v = { pipeline: this.device.createRenderPipeline(desc), layouts };
    } catch (e) {
      this.log(`pipeline ${desc.label}: ${e.message}`);
      v = null;
    }
    p.variants.set(key, v);
    return v;
  }

  // the uniform buffer, big enough for `size` bytes of a command buffer's pushes
  uniforms(size) {
    const need = Math.max(65536, size + 65536);
    if (!this.uniformBuffer || this.uniformSize < need) {
      if (this.uniformBuffer) this.uniformBuffer.destroy();
      this.uniformSize = Math.max(need, this.uniformSize * 2);
      this.uniformBuffer = this.device.createBuffer({ size: this.uniformSize, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
      this.uniformGroups.clear();
    }
  }

  uniformGroup(layouts, group) {
    const key = `${layouts.serial}/${group}`;
    let g = this.uniformGroups.get(key);
    if (!g) {
      const entries = layouts.entries[group].map((e) => ({ binding: e.binding, resource: { buffer: this.uniformBuffer, offset: 0, size: e.buffer.minBindingSize } }));
      g = this.device.createBindGroup({ layout: layouts.groups[group], entries });
      this.uniformGroups.set(key, g);
    }
    return g;
  }

  // a texture's view of one level and layer, to draw into or copy
  viewOf(t, mip, layer, format) {
    if (t === 'swapchain') {
      if (!this.current) this.current = this.context.getCurrentTexture();
      const f = this.srgb ? `${this.format}-srgb` : this.format;
      return this.current.createView({ format: f });
    }
    const key = `${mip}/${layer}/${format ?? ''}`;
    let v = t.views.get(key);
    if (!v) {
      v = t.texture.createView({ baseMipLevel: mip, mipLevelCount: 1, baseArrayLayer: layer, arrayLayerCount: 1, dimension: '2d', format: format ?? t.format });
      t.views.set(key, v);
    }
    return v;
  }

  // the mipmaps of a texture, each level drawn from the one before
  mipmaps(encoder, t) {
    if (t.levels <= 1 || t.depth) return;
    let pipe = this.mipPipelines.get(t.format);
    if (!pipe) {
      const module = this.device.createShaderModule({
        code: `
          struct V { @builtin(position) p: vec4f, @location(0) uv: vec2f }
          @vertex fn vs(@builtin(vertex_index) i: u32) -> V {
            let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
            return V(vec4f(uv * vec2f(2, -2) + vec2f(-1, 1), 0, 1), uv);
          }
          @group(0) @binding(0) var t: texture_2d<f32>;
          @group(0) @binding(1) var s: sampler;
          @fragment fn fs(v: V) -> @location(0) vec4f { return textureSampleLevel(t, s, v.uv, 0); }`,
      });
      pipe = this.device.createRenderPipeline({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: 'fs', targets: [{ format: t.format }] }, primitive: { topology: 'triangle-list' } });
      this.mipPipelines.set(t.format, pipe);
    }
    for (let layer = 0; layer < (t.kind === '3d' ? 1 : t.layers); layer++) {
      for (let level = 1; level < t.levels; level++) {
        const src = t.texture.createView({ baseMipLevel: level - 1, mipLevelCount: 1, baseArrayLayer: layer, arrayLayerCount: 1, dimension: '2d' });
        const group = this.device.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: [{ binding: 0, resource: src }, { binding: 1, resource: this.mipSampler }] });
        const pass = encoder.beginRenderPass({ colorAttachments: [{ view: this.viewOf(t, level, layer), loadOp: 'clear', storeOp: 'store', clearValue: [0, 0, 0, 0] }] });
        pass.setPipeline(pipe);
        pass.setBindGroup(0, group);
        pass.draw(3);
        pass.end();
      }
    }
  }

  // plays a command buffer's words (see gpu.rs's `op`)
  submit(wp, count, up, ulen) {
    const buf = this.memory.buffer;
    const w = new Uint32Array(buf, wp, count);
    const f = new Float32Array(buf, wp, count);
    const i32 = new Int32Array(buf, wp, count);
    const device = this.device;
    const queue = device.queue;
    try {
      if (ulen > 0) {
        this.uniforms(ulen);
        queue.writeBuffer(this.uniformBuffer, 0, buf, up, (ulen + 3) & ~3);
      }
      let encoder = device.createCommandEncoder();
      let recorded = false;
      // an upload after commands keeps its place: what came before goes first
      const flush = () => {
        if (recorded) {
          queue.submit([encoder.finish()]);
          encoder = device.createCommandEncoder();
          recorded = false;
        }
      };
      let pass = null;
      let pipe = null; // the pipeline bound, and its variant
      let variant = null;
      const offsets = [[], []]; // each stage's uniform slots: their data's offset
      let fragTex = []; // [texture id, sampler id] by slot
      let vertTex = [];
      let groupsDirty = true;
      let passSize = [0, 0];
      let k = 0;
      const textureOf = (id) => (id === SWAPCHAIN ? 'swapchain' : this.textures[id]);
      const bindFor = (draw) => {
        if (!pipe) return false;
        // which bound fragment textures can't be filtered: the pipeline's variant for them
        const loose = new Set();
        for (const t of pipe.fs.info.textures) {
          const pair = fragTex[t.binding >> 1];
          const tex = pair && this.textures[pair[0]];
          if (tex && tex.unfilterable && t.kind === 'float') loose.add(t.binding);
        }
        const v = this.variant(pipe, loose);
        if (!v) return false;
        if (v !== variant) {
          pass.setPipeline(v.pipeline);
          variant = v;
          groupsDirty = true;
        }
        const L = v.layouts;
        if (groupsDirty) {
          for (const [g, shader, bound] of [[0, pipe.vs, vertTex], [2, pipe.fs, fragTex]]) {
            const e = L.entries[g];
            if (!e.length) {
              pass.setBindGroup(g, this.emptyGroup);
              continue;
            }
            let key = `${L.serial}/${g}`;
            const resources = [];
            let ok = true;
            for (const entry of e) {
              const pair = bound[entry.binding >> 1];
              if (!pair) {
                ok = false;
                break;
              }
              if (entry.texture) {
                const t = this.textures[pair[0]];
                if (!t) {
                  ok = false;
                  break;
                }
                key += `/t${t.serial}`;
                resources.push({ binding: entry.binding, resource: t.view });
              } else {
                const s = this.samplers[pair[1]];
                if (!s) {
                  ok = false;
                  break;
                }
                const loose = entry.sampler.type === 'non-filtering';
                key += `/s${s.serial}${loose ? 'n' : ''}`;
                resources.push({ binding: entry.binding, resource: loose ? this.nonFiltering(s) : s.sampler });
              }
            }
            if (!ok) return false;
            let group = this.bindGroups.get(key);
            if (!group) {
              group = device.createBindGroup({ layout: L.groups[g], entries: resources });
              if (this.bindGroups.size > 4096) this.bindGroups.clear();
              this.bindGroups.set(key, group);
            }
            pass.setBindGroup(g, group);
          }
          groupsDirty = false;
        }
        for (const [g, stage] of [[1, 0], [3, 1]]) {
          const e = L.entries[g];
          if (!e.length) {
            pass.setBindGroup(g, this.emptyGroup);
            continue;
          }
          const dyn = e.map((entry) => offsets[stage][entry.binding] ?? 0);
          pass.setBindGroup(g, this.uniformGroup(L, g), dyn);
        }
        return true;
      };
      while (k < count) {
        const op = w[k++];
        switch (op) {
          case 1: {
            // BEGIN_RENDER
            const n = w[k++];
            const colorAttachments = [];
            for (let c = 0; c < n; c++) {
              const [tex, mip, layer, load, store] = [w[k], w[k + 1], w[k + 2], w[k + 3], w[k + 4]];
              const clear = [f[k + 5], f[k + 6], f[k + 7], f[k + 8]];
              const [rtex, rmip, rlayer] = [w[k + 9], w[k + 10], w[k + 11]];
              k += 12;
              const t = textureOf(tex);
              if (!t) continue;
              const a = { view: this.viewOf(t, mip, layer), loadOp: load === 0 ? 'load' : 'clear', clearValue: clear, storeOp: store === 0 || store === 3 ? 'store' : 'discard' };
              if ((store === 2 || store === 3) && rtex) {
                const r = textureOf(rtex);
                if (r) a.resolveTarget = this.viewOf(r, rmip, rlayer);
              }
              colorAttachments.push(a);
              if (c === 0) passSize = t === 'swapchain' ? [this.canvas.width, this.canvas.height] : [Math.max(1, t.texture.width >> mip), Math.max(1, t.texture.height >> mip)];
            }
            const desc = { colorAttachments };
            if (w[k++] === 1) {
              const [tex, load, store, depth, sload, sstore, stencil] = [w[k], w[k + 1], w[k + 2], f[k + 3], w[k + 4], w[k + 5], w[k + 6]];
              const [mip, layer] = [w[k + 7], w[k + 8]];
              k += 9;
              const t = this.textures[tex];
              if (t) {
                const z = { view: this.viewOf(t, mip, layer), depthLoadOp: load === 0 ? 'load' : 'clear', depthClearValue: depth, depthStoreOp: store === 0 || store === 3 ? 'store' : 'discard' };
                if (t.format.includes('stencil')) Object.assign(z, { stencilLoadOp: sload === 0 ? 'load' : 'clear', stencilClearValue: stencil, stencilStoreOp: sstore === 0 ? 'store' : 'discard' });
                desc.depthStencilAttachment = z;
                if (!colorAttachments.length) passSize = [Math.max(1, t.texture.width >> mip), Math.max(1, t.texture.height >> mip)];
              }
            }
            pass = encoder.beginRenderPass(desc);
            recorded = true;
            pipe = null;
            variant = null;
            fragTex = [];
            vertTex = [];
            groupsDirty = true;
            break;
          }
          case 2: // END_RENDER
            if (pass) pass.end();
            pass = null;
            break;
          case 3: // PIPELINE
            pipe = this.pipelines[w[k++]];
            variant = null;
            groupsDirty = true;
            break;
          case 4: {
            // VERTEX_BUFFERS
            const first = w[k++];
            const n = w[k++];
            for (let b = 0; b < n; b++) {
              const buffer = this.buffers[w[k]];
              const slot = first + b;
              if (buffer && pass) pass.setVertexBuffer(slot, buffer, w[k + 1]);
              k += 2;
            }
            break;
          }
          case 5: {
            // INDEX_BUFFER
            const buffer = this.buffers[w[k]];
            if (buffer && pass) pass.setIndexBuffer(buffer, w[k + 2] === 32 ? 'uint32' : 'uint16', w[k + 1]);
            k += 3;
            break;
          }
          case 6:
          case 7: {
            // FRAGMENT_TEXTURES, VERTEX_TEXTURES
            const first = w[k++];
            const n = w[k++];
            const bound = op === 6 ? fragTex : vertTex;
            for (let b = 0; b < n; b++) {
              bound[first + b] = [w[k], w[k + 1]];
              k += 2;
            }
            groupsDirty = true;
            break;
          }
          case 8: {
            // UNIFORM: stage, slot, offset, size
            offsets[w[k]][w[k + 1]] = w[k + 2];
            k += 4;
            break;
          }
          case 9: {
            // DRAW
            if (pass && bindFor()) pass.draw(w[k], w[k + 1], w[k + 2], w[k + 3]);
            k += 4;
            break;
          }
          case 10: {
            // DRAW_INDEXED
            if (pass && bindFor()) pass.drawIndexed(w[k], w[k + 1], w[k + 2], i32[k + 3], w[k + 4]);
            k += 5;
            break;
          }
          case 11: {
            // VIEWPORT
            if (pass) {
              let [x, y, vw, vh, zmin, zmax] = [f[k], f[k + 1], f[k + 2], f[k + 3], f[k + 4], f[k + 5]];
              // WebGPU keeps a viewport within its target
              const x0 = Math.max(0, x), y0 = Math.max(0, y);
              const x1 = Math.min(passSize[0], x + vw), y1 = Math.min(passSize[1], y + vh);
              if (x1 > x0 && y1 > y0) pass.setViewport(x0, y0, x1 - x0, y1 - y0, Math.max(0, Math.min(zmin, 1)), Math.max(0, Math.min(zmax, 1)));
            }
            k += 6;
            break;
          }
          case 12: {
            // SCISSOR
            if (pass) {
              const x = Math.min(w[k], passSize[0]), y = Math.min(w[k + 1], passSize[1]);
              pass.setScissorRect(x, y, Math.min(w[k + 2], passSize[0] - x), Math.min(w[k + 3], passSize[1] - y));
            }
            k += 4;
            break;
          }
          case 13: {
            // UPLOAD_BUFFER: data, size, buffer, offset
            const buffer = this.buffers[w[k + 2]];
            if (buffer && w[k + 1] > 0) {
              flush();
              queue.writeBuffer(buffer, w[k + 3], buf, w[k], (w[k + 1] + 3) & ~3);
            }
            k += 4;
            break;
          }
          case 14: {
            // UPLOAD_TEXTURE: data, length, bytes per row, rows per layer, texture, level, layer, x, y, z, w, h, d
            const t = this.textures[w[k + 4]];
            if (t && w[k + 1] > 0) {
              flush();
              const is3d = t.kind === '3d';
              queue.writeTexture(
                { texture: t.texture, mipLevel: w[k + 5], origin: { x: w[k + 7], y: w[k + 8], z: is3d ? w[k + 9] : w[k + 6] } },
                buf,
                { offset: w[k], bytesPerRow: w[k + 2], rowsPerImage: w[k + 3] },
                { width: w[k + 10], height: w[k + 11], depthOrArrayLayers: w[k + 12] },
              );
            }
            k += 13;
            break;
          }
          case 15: {
            // COPY_TEXTURE
            const s = textureOf(w[k]);
            const d = textureOf(w[k + 6]);
            if (s && d && s !== 'swapchain' && d !== 'swapchain') {
              encoder.copyTextureToTexture(
                { texture: s.texture, mipLevel: w[k + 1], origin: { x: w[k + 3], y: w[k + 4], z: s.kind === '3d' ? w[k + 5] : w[k + 2] } },
                { texture: d.texture, mipLevel: w[k + 7], origin: { x: w[k + 9], y: w[k + 10], z: d.kind === '3d' ? w[k + 11] : w[k + 8] } },
                { width: w[k + 12], height: w[k + 13], depthOrArrayLayers: w[k + 14] },
              );
              recorded = true;
            }
            k += 15;
            break;
          }
          case 16: {
            // MIPMAPS
            const t = this.textures[w[k++]];
            if (t) {
              this.mipmaps(encoder, t);
              recorded = true;
            }
            break;
          }
          default:
            this.log(`gpu: unknown command ${op} at ${k - 1}`);
            k = count;
        }
      }
      if (pass) pass.end();
      queue.submit([encoder.finish()]);
      return 1;
    } catch (e) {
      if (this.errors++ < 20) this.log(`gpu submit: ${e.stack || e}`);
      return 0;
    }
  }

  // the frame shown: the next takes the canvas's next texture
  presented() {
    this.current = null;
  }
}
Gpu.serial = 0;
