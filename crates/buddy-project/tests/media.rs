use buddy_project::*;
use image::{Delay, Frame, ImageFormat, RgbImage, Rgba, RgbaImage};
use std::{collections::BTreeMap, fs, io::Cursor, path::Path};

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/typing-cat");
    for name in [
        "buddy.project.json",
        "character/manifest.json",
        "character/sprite.png",
        "character/LICENSE.txt",
        "assets/app-icon.png",
    ] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(source.join(name), path).unwrap();
    }
    dir
}
fn gif() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for (color, delay) in [([255, 0, 0, 255], 80), ([0, 0, 255, 255], 240)] {
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(16, 16, Rgba(color)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    bytes
}
#[test]
fn static_formats_and_gif_decode_to_deterministic_pngs_with_source_timing() {
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::WebP,
        ImageFormat::Bmp,
    ] {
        let mut bytes = Cursor::new(Vec::new());
        RgbImage::from_pixel(20, 30, image::Rgb([23, 150, 80]))
            .write_to(&mut bytes, format)
            .unwrap();
        let media = decode_media(bytes.get_ref(), &mut 0, &mut BTreeMap::new()).unwrap();
        assert_eq!((media.width, media.height, media.frames.len()), (20, 30, 1));
    }
    let mut assets = BTreeMap::new();
    let media = decode_media(&gif(), &mut 0, &mut assets).unwrap();
    assert_eq!(
        media
            .frames
            .iter()
            .map(|f| f.duration_ms)
            .collect::<Vec<_>>(),
        vec![80.0, 240.0]
    );
    assert_eq!(assets.len(), 2);
    let again = decode_media(&gif(), &mut 0, &mut BTreeMap::new()).unwrap();
    assert_eq!(media.frames[0].image, again.frames[0].image);
    assert_eq!(
        image::load_from_memory(&assets[&media.frames[1].image])
            .unwrap()
            .into_rgba8()
            .get_pixel(0, 0)
            .0,
        [0, 0, 255, 255]
    );
}
#[test]
fn resources_crops_hashes_and_gif_timing_survive_loading() {
    let dir = fixture();
    fs::write(dir.path().join("character/happy.gif"), gif()).unwrap();
    let manifest = dir.path().join("character/manifest.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    c["resources"] = serde_json::json!({"happy":{"image":"happy.gif"}});
    c["animations"]["celebrate"] = serde_json::json!({"resource":"happy","fps":12,"loop":false});
    c["animations"]["idle"] = serde_json::json!({"regions":[{"x":256,"y":256,"width":256,"height":256}],"fps":2,"loop":true});
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    let loaded = load_project(dir.path()).unwrap();
    assert!(loaded.files.contains_key("character/happy.gif"));
    let clip = &loaded.data.render_plan.animations["celebrate"];
    assert_eq!(clip.frames.len(), 2);
    assert_eq!(
        clip.frames.iter().map(|f| f.duration_ms).sum::<f64>(),
        320.0
    );
    assert_eq!(
        loaded.data.render_plan.animations["idle"].frames[0]
            .region
            .x,
        256
    );
    assert!(!loaded.render_assets.is_empty());
    c["animations"]["celebrate"]["frames"] = serde_json::json!([1, 0, 1]);
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    let selected = load_project(dir.path()).unwrap();
    let frames = &selected.data.render_plan.animations["celebrate"].frames;
    assert_eq!(
        frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(),
        [240.0, 80.0, 240.0]
    );
    assert_eq!(frames[0].image, clip.frames[1].image);
    assert_eq!(frames[1].image, clip.frames[0].image);
    assert_eq!(frames[2].image, clip.frames[1].image);
    let prepared = dir.path().join("prepared-render");
    write_prepared_rendering(&selected, &prepared).unwrap();
    let packaged = load_project_with_prepared(dir.path(), &prepared).unwrap();
    let player = load_player_project(dir.path(), &prepared).unwrap();
    assert_eq!(player.content_hash, packaged.content_hash);
    assert_eq!(player.prepared_render_paths, packaged.prepared_render_paths);
    assert!(player.render_assets.is_empty());
    assert!(player.files.values().all(|asset| asset.bytes.is_empty()));
    let sprite = "character/sprite.png";
    assert_eq!(
        player.files[sprite].read_verified().unwrap(),
        packaged.files[sprite].bytes
    );
    assert!(packaged.render_assets.is_empty());
    assert_eq!(
        serde_json::to_value(&packaged.data.render_plan).unwrap(),
        serde_json::to_value(&selected.data.render_plan).unwrap()
    );
    for (name, path) in &packaged.prepared_render_paths {
        assert_eq!(fs::read(path).unwrap(), selected.render_assets[name]);
    }
    fs::write(prepared.join(&frames[0].image), b"damaged frame").unwrap();
    let recovered = load_project_with_prepared(dir.path(), &prepared).unwrap();
    assert!(recovered.prepared_render_paths.is_empty());
    assert_eq!(recovered.render_assets, selected.render_assets);
    let recovered_player = load_player_project(dir.path(), &prepared).unwrap();
    assert!(recovered_player.prepared_render_paths.is_empty());
    assert_eq!(recovered_player.render_assets, selected.render_assets);
    assert!(recovered_player
        .files
        .values()
        .all(|asset| asset.bytes.is_empty()));

    write_prepared_rendering(&selected, &prepared).unwrap();
    fs::write(dir.path().join("character/LICENSE.txt"), "updated license").unwrap();
    let changed = load_project_with_prepared(dir.path(), &prepared).unwrap();
    assert!(changed.prepared_render_paths.is_empty());
    assert_ne!(changed.content_hash, selected.content_hash);
    c["animations"]["idle"]["regions"][0]["x"] = 1000.into();
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("outside image bounds"));
}
#[test]
fn a_gif_uses_one_visible_crop_for_the_entire_selected_motion() {
    let dir = fixture();
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for x in [4, 8] {
            let mut image = RgbaImage::new(16, 20);
            for y in 3..10 {
                image.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(80, 1),
                ))
                .unwrap();
        }
    }
    fs::write(dir.path().join("character/padded.gif"), bytes).unwrap();
    let manifest = dir.path().join("character/manifest.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    c["resources"] = serde_json::json!({"padded":{"image":"padded.gif"}});
    c["animations"]["idle"] = serde_json::json!({"resource":"padded","loop":true});
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    let loaded = load_project(dir.path()).unwrap();
    let frames = &loaded.data.render_plan.animations["idle"].frames;
    for frame in frames {
        let crop = frame.region;
        assert_eq!((crop.x, crop.y, crop.width, crop.height), (4, 3, 5, 7));
    }
}
#[test]
fn decoded_budget_and_resource_path_escape_are_rejected() {
    let bytes = gif();
    let mut budget = 128 * 1024 * 1024 - 2 * 16 * 16 * 4;
    let mut assets = BTreeMap::new();
    decode_media(&bytes, &mut budget, &mut assets).unwrap();
    assert_eq!(budget, 128 * 1024 * 1024);
    assert!(decode_media(&bytes, &mut budget, &mut assets)
        .unwrap_err()
        .contains("128 MiB"));
    let dir = fixture();
    let manifest = dir.path().join("character/manifest.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    c["resources"] = serde_json::json!({"escape":{"image":"../../outside.gif"}});
    fs::write(manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("UNSAFE_PATH"));
}

#[test]
fn project_height_uses_idle_ink_and_placement_reserves_larger_mirrored_motions() {
    let dir = fixture();
    let character = dir.path().join("character");
    let mut idle = RgbaImage::new(32, 32);
    for y in 8..24 {
        for x in 8..16 {
            idle.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    idle.save(character.join("small.png")).unwrap();
    RgbaImage::from_pixel(32, 8, Rgba([255, 255, 255, 255]))
        .save(character.join("wide.png"))
        .unwrap();
    let manifest = character.join("manifest.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    c["anchor"] = serde_json::json!({"x": 0.25, "y": 1.0});
    c["resources"] =
        serde_json::json!({"small":{"image":"small.png"}, "wide":{"image":"wide.png"}});
    c["animations"]["idle"] = serde_json::json!({"resource":"small","scalePercent":50,"loop":true});
    c["animations"]["jump"] =
        serde_json::json!({"resource":"wide","scalePercent":200,"loop":false});
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    let project = dir.path().join("buddy.project.json");
    let mut p: serde_json::Value = serde_json::from_slice(&fs::read(&project).unwrap()).unwrap();
    p["defaults"]["scale"] = 0.5.into();
    fs::write(&project, serde_json::to_vec(&p).unwrap()).unwrap();
    let legacy = load_project(dir.path()).unwrap();
    let (idle_min, idle_max) = legacy.footprints.idle.unwrap();
    let (all_min, all_max) = legacy.footprints.all.unwrap();
    assert_eq!(legacy.idle_height_at_scale_one(), 128.0);
    assert_eq!(legacy.canvas_size(legacy.default_scale()).y, 256.0);
    assert!(all_min.x < idle_min.x && all_max.x > idle_max.x);
    assert_eq!((all_min.x, all_max.x), (0.0, 1.0));
    p["defaults"]["displayHeightDip"] = 72.into();
    fs::write(&project, serde_json::to_vec(&p).unwrap()).unwrap();
    let absolute = load_project(dir.path()).unwrap();
    let height = absolute.canvas_size(absolute.default_scale()).y * (idle_max.y - idle_min.y);
    assert_eq!(height, 72.0);
    assert_eq!(
        absolute.canvas_size(0.5).y * (idle_max.y - idle_min.y),
        36.0
    );

    c["animations"]["jump"]["anchorX"] = 0.0.into();
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    let offset = load_project(dir.path()).unwrap();
    let size = offset.canvas_size(1.0);
    assert_eq!(size.x, 2.0 * size.y);
    let (left, right) = offset.footprints.all.unwrap();
    assert_eq!((left.x, right.x), (0.0, 1.0));
    c["animations"]["jump"]["anchorX"] = 1.1.into();
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("anchorX"));
    c["animations"]["jump"]
        .as_object_mut()
        .unwrap()
        .remove("anchorX");
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();

    p["defaults"]["displayHeightDip"] = 0.into();
    fs::write(&project, serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("displayHeightDip"));
}
