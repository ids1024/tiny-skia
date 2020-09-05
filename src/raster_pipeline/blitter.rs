// Copyright 2016 Google Inc.
// Copyright 2020 Evgeniy Reizner
//
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::ffi::c_void;

use crate::{
    Paint, BlendMode, LengthU32, ScreenIntRect, Pixmap, PremultipliedColorU8,
    AlphaU8, PaintSource,
};
use crate::{ALPHA_U8_OPAQUE, ALPHA_U8_TRANSPARENT};

use crate::blitter::Blitter;
use crate::raster_pipeline::{self, RasterPipeline, RasterPipelineBuilder, ContextStorage};
use crate::shaders::StageRec;


pub struct RasterPipelineBlitter {
    blend_mode: BlendMode,
    color_pipeline: RasterPipelineBuilder,

    // Always points to the top-left of `pixmap`.
    img_ctx: raster_pipeline::MemoryCtx,

    memset2d_color: Option<PremultipliedColorU8>,

    // Built lazily on first use.
    blit_anti_h_rp: Option<RasterPipeline>,
    blit_rect_rp: Option<RasterPipeline>,

    // These values are pointed to by the blit pipelines above,
    // which allows us to adjust them from call to call.
    current_coverage: f32,
}

impl RasterPipelineBlitter {
    pub fn new(
        paint: &Paint,
        ctx_storage: &mut ContextStorage,
        pixmap: &mut Pixmap,
    ) -> Option<RasterPipelineBlitter> {
        let mut shader_pipeline = RasterPipelineBuilder::new();
        match paint.source {
            PaintSource::SolidColor(ref color) => {
                // Having no shader makes things nice and easy... just use the paint color.
                let color_ctx = ctx_storage.create_uniform_color_context(color.premultiply());
                shader_pipeline.push_with_context(raster_pipeline::Stage::UniformColor, color_ctx);

                let is_constant = true;
                RasterPipelineBlitter::new_inner(paint, &shader_pipeline, color.is_opaque(),
                                                 is_constant, pixmap)
            }
            PaintSource::Shader(ref shader) => {
                let is_opaque = shader.is_opaque();
                let is_constant = false;

                let rec = StageRec {
                    ctx_storage,
                    pipeline: &mut shader_pipeline,
                };

                if shader.push_stages(rec) {
                    RasterPipelineBlitter::new_inner(paint, &shader_pipeline, is_opaque,
                                                     is_constant, pixmap)
                } else {
                    None
                }
            }
        }
    }

    fn new_inner(
        paint: &Paint,
        shader_pipeline: &RasterPipelineBuilder,
        is_opaque: bool,
        is_constant: bool,
        pixmap: &mut Pixmap,
    ) -> Option<Self> {
        // Fast-reject.
        // This is basically SkInterpretXfermode().
        match paint.blend_mode {
            // `Destination` keep the pixmap unchanged. Nothing to do here.
            BlendMode::Destination => return None,
            BlendMode::DestinationIn if is_opaque && !paint.is_shader() => return None,
            _ => {}
        }

        // Our job in this factory is to fill out the blitter's color pipeline.
        // This is the common front of the full blit pipelines, each constructed lazily on first use.
        // The full blit pipelines handle reading and writing the dst, blending, coverage, dithering.
        let mut color_pipeline = RasterPipelineBuilder::new();
        color_pipeline.set_force_hq_pipeline(paint.force_hq_pipeline);

        // Let's get the shader in first.
        color_pipeline.extend(shader_pipeline);

        // We can strength-reduce SrcOver into Src when opaque.
        let mut blend_mode = paint.blend_mode;
        if is_opaque && blend_mode == BlendMode::SourceOver {
            blend_mode = BlendMode::Source;
        }

        // When we're drawing a constant color in Source mode, we can sometimes just memset.
        let mut memset2d_color = None;
        if is_constant && blend_mode == BlendMode::Source {
            // Unlike Skia, our shader cannot be constant.
            // Therefore there is no need to run a raster pipeline to get shader's color.
            if let PaintSource::SolidColor(ref color) = paint.source {
                memset2d_color = Some(color.premultiply().to_color_u8());
            }
        };

        // Clear is just a transparent color memset.
        if blend_mode == BlendMode::Clear {
            blend_mode = BlendMode::Source;
            memset2d_color = Some(PremultipliedColorU8::TRANSPARENT);
        }

        let img_ctx = raster_pipeline::MemoryCtx {
            pixels: pixmap.data().as_ptr() as _,
            stride: pixmap.size().width_safe(),
        };

        Some(RasterPipelineBlitter {
            blend_mode,
            color_pipeline,
            img_ctx,
            memset2d_color,
            blit_anti_h_rp: None,
            blit_rect_rp: None,
            current_coverage: 0.0,
        })
    }
}

impl Blitter for RasterPipelineBlitter {
    fn blit_h(&mut self, x: u32, y: u32, width: LengthU32) {
        let one = unsafe { LengthU32::new_unchecked(1) };
        let r = ScreenIntRect::from_xywh_safe(x, y, width, one);
        self.blit_rect(r);
    }

    fn blit_anti_h(&mut self, mut x: u32, y: u32, aa: &[AlphaU8], runs: &[u16]) {
        if self.blit_anti_h_rp.is_none() {
            let ctx_ptr = &self.img_ctx as *const _ as *const c_void;
            let curr_cov_ptr = &self.current_coverage as *const _ as *const c_void;

            let mut p = RasterPipelineBuilder::new();
            p.set_force_hq_pipeline(self.color_pipeline.is_force_hq_pipeline());
            p.extend(&self.color_pipeline);
            if self.blend_mode.should_pre_scale_coverage(false) {
                p.push_with_context(raster_pipeline::Stage::Scale1Float, curr_cov_ptr);
                p.push_with_context(raster_pipeline::Stage::LoadDestination, ctx_ptr);
                if let Some(blend_stage) = self.blend_mode.to_stage() {
                    p.push(blend_stage);
                }
            } else {
                p.push_with_context(raster_pipeline::Stage::LoadDestination, ctx_ptr);
                if let Some(blend_stage) = self.blend_mode.to_stage() {
                    p.push(blend_stage);
                }

                p.push_with_context(raster_pipeline::Stage::Lerp1Float, curr_cov_ptr);
            }

            p.push_with_context(raster_pipeline::Stage::Store, ctx_ptr);

            self.blit_anti_h_rp = Some(p.compile());
        }

        let mut aa_offset = 0;
        let mut run_offset = 0;
        let mut run = runs[0];
        while run > 0 {
            match aa[aa_offset] {
                ALPHA_U8_TRANSPARENT => {}
                ALPHA_U8_OPAQUE => {
                    let w = unsafe { LengthU32::new_unchecked(run as u32) };
                    self.blit_h(x, y, w);
                }
                alpha => {
                    self.current_coverage = alpha as f32 * (1.0 / 255.0);

                    let rect = unsafe {
                        ScreenIntRect::from_xywh_unchecked(x, y, u32::from(run), 1)
                    };

                    self.blit_anti_h_rp.as_ref().unwrap().run(rect);
                }
            }

            x += u32::from(run);
            run_offset += usize::from(run);
            aa_offset += usize::from(run);
            run = runs[run_offset];
        }
    }

    fn blit_rect(&mut self, rect: ScreenIntRect) {
        // TODO: reject out of bounds access

        if let Some(c) = self.memset2d_color {
            for y in 0..rect.height() {
                // Cast pixmap data to color.
                let mut addr = self.img_ctx.pixels.cast::<PremultipliedColorU8>();

                // Calculate pixel offset in bytes.
                let offset = calc_pixel_offset(rect.x(), rect.y() + y, self.img_ctx.stride.get());
                addr = unsafe { addr.add(offset) };

                for _ in 0..rect.width() as usize {
                    unsafe {
                        *addr = c;
                        addr = addr.add(1);
                    }
                }
            }

            return;
        }

        if self.blit_rect_rp.is_none() {
            let mut p = RasterPipelineBuilder::new();
            p.set_force_hq_pipeline(self.color_pipeline.is_force_hq_pipeline());
            p.extend(&self.color_pipeline);

            let ctx_ptr = &self.img_ctx as *const _ as *const c_void;

            if self.blend_mode == BlendMode::SourceOver {
                // TODO: ignore when dither_rate is non-zero
                p.push_with_context(raster_pipeline::Stage::SourceOverRgba, ctx_ptr);
            } else {
                if self.blend_mode != BlendMode::Source {
                    p.push_with_context(raster_pipeline::Stage::LoadDestination, ctx_ptr);
                    if let Some(blend_stage) = self.blend_mode.to_stage() {
                        p.push(blend_stage);
                    }
                }

                p.push_with_context(raster_pipeline::Stage::Store, ctx_ptr);
            }

            self.blit_rect_rp = Some(p.compile());
        }

        self.blit_rect_rp.as_ref().unwrap().run(rect);
    }
}

fn calc_pixel_offset(x: u32, y: u32, stride: u32) -> usize {
    calc_pixel_offset_usize(x as usize, y as usize, stride as usize)
}

fn calc_pixel_offset_usize(x: usize, y: usize, stride: usize) -> usize {
    y * stride + x
}
