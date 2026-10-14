# The RE3 span JIT (`gr2-jit`)

`src/dev/gr2/re3_jit` compiles RE3 SHADED / FLAT / FLAT4 spans. Its
pattern follows `gr4-jit` (rules/mgras/gr4-jit.md: key, store, compiler
thread, sweep test). These are the GR2-specific gotchas.

## The ABI is the `Re3` layout

A shader's only argument is `*mut Re3`. It reads registers, iterators and
private-op state at `offset_of!(Re3, ctx...)`, and VRAM and the Z buffer at
`offset_of!(Re3, vram/zbuf)`. Reordering `Re3` or `Re3Context` is safe,
because offsets are computed at compile time. Changing what a field means
is not safe unless the emitter changes too.

## Invalidation

A memoised shader is only dropped when key-shaping state changes:

- the registers in `re3_jit::shapes_key` (FUNC, ALIGNPAT, ENABPAT,
  ENABDITH, ENABWID, DEPTHFN, FBOPTION);
- the `Re3::set_pixfmt / set_zctl / set_stencil / set_blend` setters.

Rules for adding state:

- **Any new state the key depends on must go through one of these.** A
  direct `re3.ctx.pixfmt = ..` would keep running the old shader.
- **Data the shader reads live needs no invalidation:** masks, scissor,
  WID, colours, stencil reference/masks, the Z write mask. It is loaded
  per span.
- **Invalidate only on an actual change.** `gl_setup` rewrites FUNC, the
  pixel format and the Z/stencil ops on every primitive, so an
  unconditional invalidate would recompute the key per primitive.

Mode 0 is off, so a zeroed `Re3` (`bare_re3`, `Box::new_zeroed`) never runs
shaders. `Gr2::new` sets `IRIS_GR2_JIT`'s mode.

## Exactness

- The emitters mirror `draw_shaded_span` / `draw_flat_span` and their
  helpers. Change the interpreter and the emitter together. Then run
  `cargo test --release --features gr2-jit --lib re3_jit`.
- The sweep compares whole VRAM, the Z buffer and the iterators. Mutating
  the XOR op, the dither threshold or stencil DECR each made it fail at
  once (checked when it landed).
- Final iterators are written in closed form: start + steps × step
  (FLAT: steps = count / run). Stepping only adds, and both sides wrap.
  The loops step only the iterators a pixel reads.
- Row spans (DX = 0x4000, DY = 0) and every FLAT span are clipped to the
  scissor once. Their x is `X + n` exactly, because load_iterators puts the
  subpixel fraction in y when |DX| = 1.0. FLAT positions are always
  `x0 + i`, whatever DX says. Other SHADED spans clip per pixel.

## Where the time goes

Profile of IRIX gltest --tribench (8-px triangles):

- **HQ2 thread: one core, saturated.** Three quarters of it is
  `raster_poly`: edge walk, per-span float setup, and about 12 FIFO
  register writes per span.
- **RE3: about half a core.** The span JIT cannot speed small triangles
  up; moving the polygon walk off HQ2 could.

Fills gained 3× in the guest (78 to 245 Mpx/s, Z-buffered 58 to 115).
