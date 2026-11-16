use crate::{checked_path, sha256, Character, LoadedProject, Point, RenderFootprints, RenderPlan};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

// Change this version when the frame or footprint preparation rules change.
const FORMAT: u32 = 1;
const MIB: u64 = 1024 * 1024;
const MAX_INDEX: u64 = 8 * MIB;
const MAX_FRAMES: u64 = crate::DECODED_LIMIT + 32 * MIB;
const INDEX: &str = "frames.json";

#[derive(Serialize, Deserialize)]
struct Prepared {
    format: u32,
    source_hash: String,
    plan: RenderPlan,
    footprints: RenderFootprints,
}

pub fn write_prepared_rendering(project: &LoadedProject, directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let plan = &project.data.render_plan;
    let images: BTreeSet<_> = plan
        .animations
        .values()
        .flat_map(|clip| clip.frames.iter().map(|frame| &frame.image))
        .collect();
    for name in images {
        let bytes = project.render_assets.get(name).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing prepared frame bytes")
        })?;
        fs::write(directory.join(name), bytes)?;
    }
    fs::write(
        directory.join(INDEX),
        serde_json::to_vec(&Prepared {
            format: FORMAT,
            source_hash: project.content_hash.clone(),
            plan: plan.clone(),
            footprints: project.footprints.clone(),
        })?,
    )
}

type FramePaths = BTreeMap<String, PathBuf>;

pub(crate) fn read_prepared_rendering(
    directory: &Path,
    source_hash: &str,
    character: &Character,
) -> Option<(RenderPlan, RenderFootprints, FramePaths)> {
    let directory = fs::canonicalize(directory).ok()?;
    let index = checked_path(&directory, INDEX).ok()?;
    let prepared: Prepared = serde_json::from_slice(&read_limited(&index, MAX_INDEX)?).ok()?;
    if prepared.format != FORMAT
        || prepared.source_hash != source_hash
        || !prepared
            .plan
            .animations
            .keys()
            .eq(character.animations.keys())
        || !valid_footprints(&prepared.footprints, character)
    {
        return None;
    }
    let mut remaining = MAX_FRAMES;
    let mut decoded = 0;
    let mut images = BTreeMap::new();
    for (name, clip) in &prepared.plan.animations {
        if !(1..=256).contains(&clip.frames.len())
            || clip.looping != character.animations[name].looping
        {
            return None;
        }
        for frame in &clip.frames {
            if !images.contains_key(&frame.image) {
                let (path, dimensions, size) = read_frame(&directory, &frame.image, remaining)?;
                remaining -= size;
                decoded += u64::from(dimensions.0) * u64::from(dimensions.1) * 4;
                if decoded > crate::DECODED_LIMIT {
                    return None;
                }
                images.insert(frame.image.clone(), (path, dimensions));
            }
            let (_, (width, height)) = images.get(&frame.image)?;
            let region = frame.region;
            if !frame.duration_ms.is_finite()
                || frame.duration_ms <= 0.0
                || region.width == 0
                || region.height == 0
                || region.x.saturating_add(region.width) > *width
                || region.y.saturating_add(region.height) > *height
            {
                return None;
            }
        }
    }
    let paths = images
        .into_iter()
        .map(|(name, (path, _))| (name, path))
        .collect();
    Some((prepared.plan, prepared.footprints, paths))
}

fn valid_footprints(footprints: &RenderFootprints, character: &Character) -> bool {
    let valid = |(min, max): (Point, Point)| {
        // Allow the last bit of floating point rounding at the canvas boundary.
        let range = -1e-9..=1.0 + 1e-9;
        range.contains(&min.x)
            && range.contains(&min.y)
            && range.contains(&max.x)
            && range.contains(&max.y)
            && min.x < max.x
            && min.y < max.y
    };
    footprints.idle
        == footprints
            .animations
            .get(&character.default_animation)
            .copied()
        && footprints.idle.is_none_or(valid)
        && footprints.all.is_none_or(valid)
        && footprints
            .animations
            .iter()
            .all(|(name, bounds)| character.animations.contains_key(name) && valid(*bounds))
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size > limit {
        return None;
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 == size).then_some(bytes)
}

fn read_frame(directory: &Path, name: &str, limit: u64) -> Option<(PathBuf, (u32, u32), u64)> {
    let hash = name.strip_suffix(".png")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let path = checked_path(directory, name).ok()?;
    let bytes = read_limited(&path, limit)?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR") || sha256(&bytes) != hash {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    ((1..=4096).contains(&width) && (1..=4096).contains(&height)).then_some((
        path,
        (width, height),
        bytes.len() as u64,
    ))
}
