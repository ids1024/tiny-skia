# tiny-skia
![Build Status](https://github.com/RazrFalcon/tiny-skia/workflows/Rust/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tiny-skia.svg)](https://crates.io/crates/tiny-skia)
[![Documentation](https://docs.rs/tiny-skia/badge.svg)](https://docs.rs/tiny-skia)
[![Rust 1.36+](https://img.shields.io/badge/rust-1.36+-orange.svg)](https://www.rust-lang.org)

`tiny-skia` is a tiny [Skia] subset ported to Rust.

The goal is to provide an absolute minimal, CPU only, 2D rendering library for the Rust ecosystem,
with a focus on a rendering quality, speed and binary size.

**Note:** this is not a Skia replacement and never will be. It's more of a research project.

## Motivation

The main motivation behind this library is to have a small, high-quality 2D rendering
library that can be used by [resvg]. And the choice is rather limited.
You basically have to choose between cairo, Qt and Skia. And all of them are
relatively bloated, hard to compile and distribute. Not to mention that none of them
is written in Rust.

But if we ignore those problems and focus only on quality and speed alone,
Skia is by far the best one.
However, the main problem with Skia is that it's huge. Really huge.
It supports CPU and GPU rendering, multiple input and output formats (including SVG and PDF),
various filters, color spaces, color types and text rendering.
It consists of 370 KLOC without dependencies (around 7 MLOC with dependencies)
and requires around 4-8 GiB of disk space to be built from sources.
And the final binary is 3-8 MiB big, depending on enabled features.
Not to mention that it requires `clang` and no other compiler,
uses an obscure build system (`gn`) which still uses Python2
and doesn't really support 32bit targets.

`tiny-skia` tries to be small, simple and easy to build.

## Performance

Does `tiny-skia` as fast as [Skia]? The short answer is no. The longer one is: it depends.

The heart of Skia's CPU rendering is
[SkRasterPipeline](https://github.com/google/skia/blob/master/src/opts/SkRasterPipeline_opts.h).
And this is an extremely optimized piece of code.
But to be a bit pedantic, it's not really a C++ code. It relies on clang's
non-standard vector extensions, which means that you must build it with clang.
You can actually build it with gcc/msvc, but it will simply ignore all the optimizations
and become 15-30 *times* slower! Which makes it kinda useless. And `tiny-skia`
is way closer to a clang version.

Also, `SkRasterPipeline` supports AVX2 instructions, which provide 256-bits wide types.
This makes common operations almost 2x faster, compared to a generic SSE2/128-bits one.
Which is no surprise.<br>
The problem is that Skia doesn't support dynamic CPU detection.
So by enabling AVX2 you're making the resulting binary non-portable,
since you need a Haswell processor or newer.<br>
Right now, `tiny-skia` supports only SSE2 instructions and relies on autovectorization.

Skia also supports ARM NEON instructions, which are unavailable in a stable Rust at the moment.
Therefore a default scalar implementation will be used instead on ARM and other non-x86 targets.

Accounting all above, `tiny-skia` is 20-100% slower than "a Skia built for a generic x86_64 CPU".

We can technically use the `SkRasterPipeline` directly, to achive the same performance as Skia has.
But it means that we have to complicate our build process quite a lot.
Mainly because we have to use only clang.
So having a pure Rust library, even a bit slower one, is still a good trade off.

You can find more information in [benches/README.md](./benches/README.md).

## API overview

The API is a bit unconventional. It doesn't look like cairo, QPainter (Qt) or even Skia.

To start, it's completely stateless. There is no canvas-like object that stores a transform,
clip, layers, etc. You only have a `Pixmap` and you can draw a `Path` on top of it. That's it.<br>
If you want to draw a transformed `Path` you should transform it first, and then you can fill it.<br>
If you want to stroke a `Path` you should create a stroked `Path` from an existing `Path`,
and then you can fill it.<br>
The main motivation behind this is that there is almost no hidden cost (like allocations).
Everything is transparent to the caller.

Another difference is that everting is strongly typed and checked on creation.
There are almost no methods that accept primive types (`i32`, `f32`, etc.).
`Size`, `Rect`, `Color`, `Path` - everything is guarantee to be valid at all times.
You cannot create a zero or negative `Size`. You cannot create an empty `Path`.
You cannot create a transform with a zero scale. And so on.<br>
Most of it is handled by externals crates like
[checked-geom](https://github.com/RazrFalcon/checked-geom)
and [num-ext](https://github.com/RazrFalcon/num-ext).

## Roadmap

### v0.2

- [x] Foundation: `Pixmap`, `Painter`, `Path`, geometry primitives, etc.
- [x] Port `SkRasterPipeline` to Rust.
- [x] PNG load/save
- [x] Blending modes
- [x] `Path` filling
- [x] Anti-aliased `Path` filling
- [ ] Analytical anti-aliased `Path` filling
- [x] `Path` stroking
- [ ] `Path` hairline stroking
- [ ] Anti-aliased `Path` hairline stroking
- [ ] Stroke dashing
- [x] Gradients (linear, radial and two point conical)
- [ ] `Pixmap`s blending (image on image rendering)
- [ ] Patterns
- [ ] Dithering

### v0.3

- [ ] Clipping
- [ ] Anti-aliased clipping

### v0.N

- Move `Path` and most of Bézier math into separate crates. Preferably into existing one

PS: we start from 0.2, because 0.1 was just a bindings.

## Notes about the port

`tiny-skia` should be viewed as a Rust 2D rendering library that uses Skia algorithms internally.
We have a completely different public API. The internals are also extremely simplified.
But all the core logic and math is borrowed from Skia. Hence the name.

As for the porting process itself, Skia uses goto, inheritance, virtual methods, linked lists,
const generics and templates specialization a lot, and all of this features are unavailable in Rust.
There are also a lot of pointers magic, implicit mutations and caches.
Therefore we have to compromise or even rewrite some parts from scratch.

## Safety

The project relies heavily on unsafe. Not much we can do about it at the moment.

## License

The same as used by [Skia]: [New BSD License](./LICENSE)

[Skia]: https://skia.org/
[resvg]: https://github.com/RazrFalcon/resvg
