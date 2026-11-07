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
    c["animations"]["idle"]["regions"][0]["x"] = 1000.into();
    fs::write(&manifest, serde_json::to_vec(&c).unwrap()).unwrap();
    assert!(load_project(dir.path())
        .unwrap_err()
        .to_string()
        .contains("outside image bounds"));
}
#[test]
fn decoded_budget_and_resource_path_escape_are_rejected() {
    let mut budget = DECODED_LIMIT;
    assert!(decode_media(&gif(), &mut budget, &mut BTreeMap::new())
        .unwrap_err()
        .contains("64 MiB"));
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
