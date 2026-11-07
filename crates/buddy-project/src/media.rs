use crate::{Animation, Character, FrameRegion, RenderAnimation, RenderFrame, RenderPlan};
use image::{AnimationDecoder, ImageDecoder, ImageFormat, RgbaImage};
use std::{collections::BTreeMap, io::Cursor};

pub const DECODED_LIMIT: u64 = 64 * 1024 * 1024;
#[derive(Debug, Clone)]
pub struct DecodedMedia {
    pub frames: Vec<RenderFrame>,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(DECODED_LIMIT);
    limits
}
/// Decode and composite GIF disposal/transparency in Rust. The renderer receives
/// ordinary PNGs, so restart, frame timing and completion share the engine clock.
pub fn decode_media(
    bytes: &[u8],
    budget: &mut u64,
    assets: &mut BTreeMap<String, Vec<u8>>,
) -> Result<DecodedMedia, String> {
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
    ) {
        return Err("supported resources: PNG, JPEG, GIF, WebP, BMP".into());
    }
    let mut frames = Vec::new();
    let mut dimensions = (0, 0);
    let animated = format == ImageFormat::Gif;
    let mut add = |buffer: RgbaImage, delay: f64| -> Result<(), String> {
        if frames.len() >= 256 {
            return Err("maximum 256 GIF frames".into());
        }
        *budget = budget.saturating_add(u64::from(buffer.width()) * u64::from(buffer.height()) * 4);
        if *budget > DECODED_LIMIT {
            return Err("project decoded images exceed 64 MiB".into());
        }
        dimensions = buffer.dimensions();
        let mut png = Cursor::new(Vec::new());
        buffer
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let bytes = png.into_inner();
        let image = format!("{}.png", crate::sha256(&bytes));
        assets.entry(image.clone()).or_insert(bytes);
        frames.push(RenderFrame {
            image,
            region: FrameRegion {
                x: 0,
                y: 0,
                width: dimensions.0,
                height: dimensions.1,
            },
            duration_ms: if delay <= 0.0 {
                100.0
            } else {
                delay.max(1000.0 / 30.0)
            },
        });
        Ok(())
    };
    if animated {
        let mut decoder =
            image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        decoder.set_limits(limits()).map_err(|e| e.to_string())?;
        for frame in decoder.into_frames() {
            let frame = frame.map_err(|e| e.to_string())?;
            let (n, d) = frame.delay().numer_denom_ms();
            add(frame.into_buffer(), f64::from(n) / f64::from(d.max(1)))?;
        }
    } else {
        // Animated WebP/APNG are intentionally rejected: source animation is GIF only.
        if (format == ImageFormat::WebP && bytes.windows(4).any(|w| w == b"ANIM"))
            || (format == ImageFormat::Png && bytes.windows(4).any(|w| w == b"acTL"))
        {
            return Err("animated WebP/APNG: convert to GIF or a sprite sheet".into());
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits());
        add(
            reader.decode().map_err(|e| e.to_string())?.into_rgba8(),
            100.0,
        )?;
    }
    if frames.is_empty() {
        return Err("image has no frames".into());
    }
    Ok(DecodedMedia {
        frames,
        width: dimensions.0,
        height: dimensions.1,
        animated,
    })
}

pub fn compile_animation(a: &Animation, media: &DecodedMedia) -> Result<RenderAnimation, String> {
    let mut frames = Vec::new();
    if !a.regions.is_empty() {
        if media.animated {
            return Err("regions apply to a static image, not GIF".into());
        }
        for region in &a.regions {
            if region.width == 0
                || region.height == 0
                || region
                    .x
                    .checked_add(region.width)
                    .is_none_or(|v| v > media.width)
                || region
                    .y
                    .checked_add(region.height)
                    .is_none_or(|v| v > media.height)
            {
                return Err("sprite region is outside image bounds".into());
            }
            let mut frame = media.frames[0].clone();
            frame.region = *region;
            frame.duration_ms = 1000.0 / f64::from(a.fps);
            frames.push(frame);
        }
    } else {
        let indices: Vec<_> = if a.frames.is_empty() {
            (0..media.frames.len() as u32).collect()
        } else {
            a.frames.clone()
        };
        for index in indices {
            let mut frame = media
                .frames
                .get(index as usize)
                .ok_or("frame index outside resource")?
                .clone();
            if !media.animated || !a.use_source_timing {
                frame.duration_ms = 1000.0 / f64::from(a.fps);
            }
            frames.push(frame);
        }
    }
    Ok(RenderAnimation {
        frames,
        looping: a.looping,
    })
}

pub fn legacy_plan(character: &Character, image: &str) -> RenderPlan {
    let s = character.sheet;
    let columns = (s.width / s.frame_width.max(1)).max(1);
    let animations = character
        .animations
        .iter()
        .filter(|(_, a)| a.resource.is_none())
        .map(|(name, a)| {
            let regions: Vec<_> = if a.regions.is_empty() {
                a.frames
                    .iter()
                    .map(|i| FrameRegion {
                        x: (i % columns) * s.frame_width,
                        y: (i / columns) * s.frame_height,
                        width: s.frame_width,
                        height: s.frame_height,
                    })
                    .collect()
            } else {
                a.regions.clone()
            };
            (
                name.clone(),
                RenderAnimation {
                    looping: a.looping,
                    frames: regions
                        .into_iter()
                        .map(|region| RenderFrame {
                            image: image.into(),
                            region,
                            duration_ms: 1000.0 / f64::from(a.fps.max(1)),
                        })
                        .collect(),
                },
            )
        })
        .collect();
    RenderPlan { animations }
}
