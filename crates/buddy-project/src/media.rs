use crate::{Animation, Character, FrameRegion, Point, RenderAnimation, RenderFrame, RenderPlan};
use image::{AnimationDecoder, ImageDecoder, ImageFormat, RgbaImage};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};

const MIB: u64 = 1024 * 1024;
const IMAGE_DECODE_ALLOC_LIMIT: u64 = 64 * MIB;
pub const DECODED_LIMIT: u64 = 128 * MIB;
#[derive(Debug, Clone)]
pub struct DecodedMedia {
    pub frames: Vec<RenderFrame>,
    visible_bounds: Vec<Option<FrameRegion>>,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

#[derive(Default)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub animated: bool,
    pub duration_ms: f64,
}

fn visible_bounds(buffer: &RgbaImage) -> Option<FrameRegion> {
    visible_bounds_in(
        buffer,
        FrameRegion {
            x: 0,
            y: 0,
            width: buffer.width(),
            height: buffer.height(),
        },
    )
}

fn visible_bounds_in(buffer: &RgbaImage, region: FrameRegion) -> Option<FrameRegion> {
    let FrameRegion {
        x,
        y,
        width,
        height,
    } = region;
    if width == 0 {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for (row_index, row) in buffer
        .as_raw()
        .chunks_exact(buffer.width() as usize * 4)
        .skip(y as usize)
        .take(height as usize)
        .enumerate()
    {
        let row = &row[x as usize * 4..(x + width) as usize * 4];
        let (pixels, _) = row.as_chunks::<4>();
        let opaque = |pixel: &[u8; 4]| pixel[3] != 0;
        if let Some(first) = pixels.iter().position(opaque) {
            let last = pixels.iter().rposition(opaque).unwrap();
            left = left.min(first as u32);
            right = right.max(last as u32 + 1);
            top = top.min(row_index as u32);
            bottom = row_index as u32 + 1;
        }
    }
    (right > left && bottom > top).then(|| FrameRegion {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// Actual painted bounds, normalized to the overlay, with both facings included.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RenderFootprints {
    pub idle: Option<(Point, Point)>,
    pub all: Option<(Point, Point)>,
    pub animations: BTreeMap<String, (Point, Point)>,
}

pub(crate) fn render_footprints<'a>(
    character: &Character,
    plan: &RenderPlan,
    decoded: impl Iterator<Item = &'a DecodedMedia>,
    assets: &BTreeMap<String, Vec<u8>>,
) -> RenderFootprints {
    let alpha: BTreeMap<_, _> = decoded
        .flat_map(|media| media.frames.iter().zip(&media.visible_bounds))
        .map(|(frame, bounds)| (frame.image.as_str(), *bounds))
        .collect();
    let mut regions = BTreeMap::<&str, BTreeSet<[u32; 4]>>::new();
    for frame in plan.animations.values().flat_map(|clip| &clip.frames) {
        regions
            .entry(&frame.image)
            .or_default()
            .insert(region_key(frame.region));
    }
    let mut bounds = BTreeMap::new();
    for (name, regions) in regions {
        let mut image = None;
        let full = alpha[name];
        for key @ [x, y, width, height] in regions {
            let ink = full.and_then(|ink| {
                if ink.x + ink.width <= x
                    || x + width <= ink.x
                    || ink.y + ink.height <= y
                    || y + height <= ink.y
                {
                    return None;
                }
                if ink.x >= x
                    && ink.y >= y
                    && ink.x + ink.width <= x + width
                    && ink.y + ink.height <= y + height
                {
                    return Some(FrameRegion {
                        x: ink.x - x,
                        y: ink.y - y,
                        ..ink
                    });
                }
                // A sheet cell may contain only part of the image's alpha bounds.
                // Decode that sheet once; full GIF frames need no second decode.
                let image = image.get_or_insert_with(|| {
                    image::load_from_memory_with_format(&assets[name], ImageFormat::Png)
                        .expect("prepared render PNG was encoded in this process")
                        .into_rgba8()
                });
                visible_bounds_in(
                    image,
                    FrameRegion {
                        x,
                        y,
                        width,
                        height,
                    },
                )
            });
            bounds.insert((name, key), ink);
        }
    }
    let mut result = RenderFootprints::default();
    let frame_width = f64::from(character.sheet.frame_width);
    let frame_height = f64::from(character.sheet.frame_height);
    let window_width = frame_width * 2.0 * character.horizontal_anchor_extent();
    let scale_envelope = character.animation_scale_envelope();
    for (name, clip) in &plan.animations {
        // Match CharacterRenderer's fit, alignment and reserved scale.
        let max_width = f64::from(
            clip.frames
                .iter()
                .map(|f| f.region.width)
                .max()
                .unwrap_or(1),
        );
        let max_height = f64::from(
            clip.frames
                .iter()
                .map(|f| f.region.height)
                .max()
                .unwrap_or(1),
        );
        let animation = &character.animations[name];
        let anchor_x = animation.anchor_x.unwrap_or(character.anchor.x);
        let scale = f64::from(animation.scale_percent.unwrap_or(100)) / 100.0;
        let fit = (frame_width / max_width).min(frame_height / max_height) * scale / scale_envelope;
        let mut footprint = None;
        for frame in &clip.frames {
            let region = frame.region;
            let Some(ink) = bounds[&(frame.image.as_str(), region_key(region))] else {
                continue;
            };
            let x = window_width / 2.0 - anchor_x * f64::from(region.width) * fit;
            let y = (frame_height - f64::from(region.height) * fit) * character.anchor.y;
            let left = (x + f64::from(ink.x) * fit) / window_width;
            let right = (x + f64::from(ink.x + ink.width) * fit) / window_width;
            merge_footprint(
                &mut footprint,
                (
                    Point {
                        x: left.min(1.0 - right),
                        y: (y + f64::from(ink.y) * fit) / frame_height,
                    },
                    Point {
                        x: right.max(1.0 - left),
                        y: (y + f64::from(ink.y + ink.height) * fit) / frame_height,
                    },
                ),
            );
        }
        if name == &character.default_animation {
            result.idle = footprint;
        }
        if let Some(footprint) = footprint {
            merge_footprint(&mut result.all, footprint);
            result.animations.insert(name.clone(), footprint);
        }
    }
    result
}

fn region_key(region: FrameRegion) -> [u32; 4] {
    [region.x, region.y, region.width, region.height]
}

fn merge_footprint(target: &mut Option<(Point, Point)>, (min, max): (Point, Point)) {
    if let Some((a, b)) = target {
        a.x = a.x.min(min.x);
        a.y = a.y.min(min.y);
        b.x = b.x.max(max.x);
        b.y = b.y.max(max.y);
    } else {
        *target = Some((min, max));
    }
}

fn union_bounds(a: FrameRegion, b: FrameRegion) -> FrameRegion {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    FrameRegion {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(IMAGE_DECODE_ALLOC_LIMIT);
    limits
}
/// Decode and composite GIF disposal/transparency in Rust. The renderer receives
/// ordinary PNGs, so restart, frame timing and completion share the engine clock.
pub fn decode_media(
    bytes: &[u8],
    budget: &mut u64,
    assets: &mut BTreeMap<String, Vec<u8>>,
) -> Result<DecodedMedia, String> {
    let mut frames = Vec::new();
    let mut bounds = Vec::new();
    let info = visit_media(bytes, budget, |_, buffer, duration_ms| {
        bounds.push(visible_bounds(buffer));
        let bytes = encode_png(buffer)?;
        let image = format!("{}.png", crate::sha256(&bytes));
        assets.entry(image.clone()).or_insert(bytes);
        frames.push(RenderFrame {
            image,
            region: FrameRegion {
                x: 0,
                y: 0,
                width: buffer.width(),
                height: buffer.height(),
            },
            duration_ms,
        });
        Ok(())
    })?;
    Ok(DecodedMedia {
        frames,
        visible_bounds: bounds,
        width: info.width,
        height: info.height,
        animated: info.animated,
    })
}

pub fn probe_media(
    bytes: &[u8],
    frame: Option<usize>,
) -> Result<(MediaInfo, Option<Vec<u8>>), String> {
    let mut png = None;
    let info = visit_media(bytes, &mut 0, |index, buffer, _| {
        if frame == Some(index) {
            png = Some(encode_png(buffer)?);
        }
        Ok(())
    })?;
    if frame.is_some() && png.is_none() {
        return Err("frame index outside resource".into());
    }
    Ok((info, png))
}

fn encode_png(buffer: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut png = Cursor::new(Vec::new());
    buffer
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png.into_inner())
}

fn visit_media(
    bytes: &[u8],
    budget: &mut u64,
    mut visit: impl FnMut(usize, &RgbaImage, f64) -> Result<(), String>,
) -> Result<MediaInfo, String> {
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
    let animated = format == ImageFormat::Gif;
    let mut info = MediaInfo {
        animated,
        ..MediaInfo::default()
    };
    let mut add = |buffer: RgbaImage, delay: f64| -> Result<(), String> {
        if info.frames >= 256 {
            return Err("maximum 256 GIF frames".into());
        }
        *budget = budget.saturating_add(u64::from(buffer.width()) * u64::from(buffer.height()) * 4);
        if *budget > DECODED_LIMIT {
            return Err(format!(
                "project decoded images exceed {} MiB",
                DECODED_LIMIT / MIB
            ));
        }
        let duration_ms = if delay <= 0.0 {
            100.0
        } else {
            delay.max(1000.0 / 30.0)
        };
        visit(info.frames, &buffer, duration_ms)?;
        (info.width, info.height) = buffer.dimensions();
        info.frames += 1;
        info.duration_ms += duration_ms;
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
    if info.frames == 0 {
        return Err("image has no frames".into());
    }
    Ok(info)
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
        let mut bounds = None;
        for index in indices {
            let mut frame = media
                .frames
                .get(index as usize)
                .ok_or("frame index outside resource")?
                .clone();
            if let Some(region) = media.visible_bounds[index as usize] {
                bounds = Some(bounds.map_or(region, |current| union_bounds(current, region)));
            }
            if !media.animated || !a.use_source_timing {
                frame.duration_ms = 1000.0 / f64::from(a.fps);
            }
            frames.push(frame);
        }
        // A shared crop removes transparent padding while preserving motion
        // and framing across all selected frames, including transparent holds.
        if let Some(region) = bounds {
            for frame in &mut frames {
                frame.region = region;
            }
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
