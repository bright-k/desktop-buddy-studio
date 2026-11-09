use crate::*;
use serde::{
    de::{DeserializeOwned, Error as _, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fmt, fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};
use unicode_normalization::UnicodeNormalization;

const JSON_LIMIT: u64 = 1024 * 1024;
const PNG_LIMIT: u64 = 16 * 1024 * 1024;
const ASSET_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub file: String,
    pub path: String,
    pub code: String,
    pub message: String,
}
#[derive(Debug)]
pub struct ValidationError(pub Vec<Diagnostic>);
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for error in &self.0 {
            writeln!(
                f,
                "{}:{} [{}] {}",
                error.file, error.path, error.code, error.message
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for ValidationError {}
fn problem(file: &str, path: &str, code: &str, message: impl Into<String>) -> ValidationError {
    ValidationError(vec![Diagnostic {
        file: file.into(),
        path: path.into(),
        code: code.into(),
        message: message.into(),
    }])
}

#[derive(Debug, Clone)]
pub struct SourceAsset {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub modified: Option<SystemTime>,
    pub hash: String,
}
#[derive(Debug, Clone)]
pub struct ValidatedProject {
    pub project: Project,
    pub character: Character,
    pub sequences: Vec<Option<Vec<KeyStep>>>,
    pub sequence_alternatives: Vec<Vec<Vec<KeyStep>>>,
    pub render_plan: RenderPlan,
}
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub root: PathBuf,
    pub entry_relative_path: String,
    pub data: ValidatedProject,
    pub files: BTreeMap<String, SourceAsset>,
    pub image_relative_path: String,
    pub content_hash: String,
    pub render_assets: BTreeMap<String, Vec<u8>>,
    /// Only populated when the native Player loads verified bundled frames.
    pub prepared_render_paths: BTreeMap<String, PathBuf>,
    pub sound_files: BTreeMap<String, String>,
    pub footprints: crate::RenderFootprints,
}

impl LoadedProject {
    pub fn idle_height_at_scale_one(&self) -> f64 {
        self.data.character.canvas_size(1.0).y
            * self.footprints.idle.map_or(1.0, |(min, max)| max.y - min.y)
    }

    pub fn default_scale(&self) -> f64 {
        let defaults = &self.data.project.defaults;
        defaults.display_height_dip.map_or(defaults.scale, |_| 1.0)
    }

    pub fn canvas_size(&self, scale: f64) -> Point {
        let height = self.data.project.defaults.display_height_dip;
        self.data
            .character
            .canvas_size(scale * height.map_or(1.0, |h| h / self.idle_height_at_scale_one()))
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Reject duplicate keys recursively before typed deserialization; serde_json::Value
// alone would silently keep the last occurrence.
struct UniqueValue(serde_json::Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate object fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut out = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(M::Error::custom(format!("duplicate JSON field: {key}")));
                    }
                    out.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(out.into()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    out.push(value.0);
                }
                Ok(UniqueValue(out.into()))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(UniqueValue(
                    serde_json::Number::from_f64(v)
                        .ok_or_else(|| E::custom("non-finite number"))?
                        .into(),
                ))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(serde_json::Value::Null))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn parse_json<T: DeserializeOwned>(bytes: &[u8], file: &str) -> Result<T, ValidationError> {
    if bytes.len() as u64 > JSON_LIMIT {
        return Err(problem(file, "$", "JSON_TOO_LARGE", "JSON exceeds 1 MiB"));
    }
    let value = serde_json::from_slice::<UniqueValue>(bytes)
        .map_err(|e| problem(file, "$", "INVALID_JSON", e.to_string()))?
        .0;
    serde_path_to_error::deserialize(value).map_err(|e| {
        problem(
            file,
            &format!("$.{}", e.path()),
            "INVALID_FIELD",
            e.inner().to_string(),
        )
    })
}

pub fn validate_parts(
    mut project: Project,
    character: Character,
) -> Result<ValidatedProject, ValidationError> {
    let mut errors = Vec::new();
    let mut check = |ok: bool, file: &str, path: &str, message: &str| {
        if !ok {
            errors.push(Diagnostic {
                file: file.into(),
                path: path.into(),
                code: "INVALID_PROJECT".into(),
                message: message.into(),
            });
        }
    };
    let file = "buddy.project.json";
    check(
        project.schema_version == 1,
        file,
        "$.schemaVersion",
        "only schemaVersion 1 is supported",
    );
    check(
        uuid::Uuid::parse_str(&project.project_id).is_ok(),
        file,
        "$.projectId",
        "expected UUID",
    );
    let product = &project.product;
    check(
        (2..=64).contains(&product.slug.len())
            && product
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && product
                .slug
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphanumeric())
            && product
                .slug
                .as_bytes()
                .last()
                .is_some_and(|b| b.is_ascii_alphanumeric())
            && portable_segment(&product.slug),
        file,
        "$.product.slug",
        "expected portable 2..64 lowercase ASCII slug",
    );
    check(
        (1..=80).contains(&product.name.chars().count())
            && product.name.len() <= 200
            && portable_segment(&product.name),
        file,
        "$.product.name",
        "expected portable product name: 1..80 characters, at most 200 UTF-8 bytes, no reserved filename characters or names",
    );
    let parts: Vec<_> = product.version.split('.').collect();
    check(
        parts.len() == 3
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0'))
                    && part.parse::<u8>().is_ok_and(|v| v <= 99)
            }),
        file,
        "$.product.version",
        "expected major.minor.patch with each component 0..99 and no leading zeros",
    );
    let segments: Vec<_> = product.identifier.split('.').collect();
    check(
        product.identifier.len() <= 128
            && segments.len() >= 3
            && segments.iter().all(|part| {
                part.as_bytes()
                    .first()
                    .is_some_and(|b| b.is_ascii_lowercase())
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            }),
        file,
        "$.product.identifier",
        "expected lowercase reverse-DNS identifier with at least 3 segments",
    );
    check(
        (0.25..=3.0).contains(&project.defaults.scale),
        file,
        "$.defaults.scale",
        "scale must be 0.25..3.0",
    );
    check(
        project
            .defaults
            .display_height_dip
            .is_none_or(|height| (16.0..=1024.0).contains(&height)),
        file,
        "$.defaults.displayHeightDip",
        "display height must be 16..1024 DIP",
    );
    check(
        project.defaults.position.monitor == "primary",
        file,
        "$.defaults.position.monitor",
        "v1 initial monitor must be primary",
    );
    check(
        (0.0..=256.0).contains(&project.defaults.position.margin_dip),
        file,
        "$.defaults.position.marginDip",
        "margin must be 0..256 DIP",
    );
    if project.build.targets.is_empty() {
        if let Some(target) = Target::current() {
            project.build.targets.push(target);
        }
    }
    let mut target_set = HashSet::new();
    check(
        project
            .build
            .targets
            .iter()
            .all(|t| target_set.insert(t.rust_target())),
        file,
        "$.build.targets",
        "targets must not contain duplicates",
    );
    check(
        project.bindings.len() <= 256,
        file,
        "$.bindings",
        "at most 256 mappings",
    );
    if let Some(dmg) = &project.build.macos_dmg {
        check(
            (400..=1600).contains(&dmg.width)
                && (260..=1200).contains(&dmg.height)
                && [dmg.app_position, dmg.applications_position]
                    .into_iter()
                    .all(|point| {
                        point.x >= 72
                            && point.y >= 72
                            && point.x <= dmg.width - 72
                            && point.y <= dmg.height - crate::MacosDmg::ICON_BOTTOM_MARGIN
                    })
                && (f64::from(dmg.app_position.x) - f64::from(dmg.applications_position.x))
                    .hypot(f64::from(dmg.app_position.y) - f64::from(dmg.applications_position.y))
                    >= 150.0,
            file,
            "$.build.macosDmg",
            "window 400..1600 × 260..1200; keep icons separated and inside the window",
        );
        check(
            dmg.guide.is_none() || dmg.guide_position().is_some(),
            file,
            "$.build.macosDmg.guidePosition",
            "keep the guide icon inside the window and separated from the other icons",
        );
    }
    let timing = project.defaults.timing;
    check(timing.is_valid(),
        file, "$.defaults.timing", "window 100..10000, evaluation 10..250, stability 0..2000, idle 50..5000 ms; evaluation <= idle");
    let char_file = project.character.as_str();
    check(
        character.schema_version == 1,
        char_file,
        "$.schemaVersion",
        "only character schemaVersion 1 is supported",
    );
    check(
        safe_id(&character.id),
        char_file,
        "$.id",
        "invalid character ID",
    );
    check(
        (1..=80).contains(&character.name.chars().count())
            && !character.name.chars().any(char::is_control),
        char_file,
        "$.name",
        "invalid character name",
    );
    let sheet = character.sheet;
    let dimensions_ok = (1..=4096).contains(&sheet.width) && (1..=4096).contains(&sheet.height);
    check(
        dimensions_ok,
        char_file,
        "$.sheet",
        "sheet dimensions must be 1..4096",
    );
    let frame_ok =
        (16..=512).contains(&sheet.frame_width) && (16..=512).contains(&sheet.frame_height);
    check(
        frame_ok,
        char_file,
        "$.sheet",
        "frame dimensions must be 16..512",
    );
    let cells_ok = dimensions_ok
        && frame_ok
        && sheet.width.is_multiple_of(sheet.frame_width)
        && sheet.height.is_multiple_of(sheet.frame_height)
        && (1..=256).contains(&sheet.frame_count)
        && sheet.frame_count
            <= (sheet.width / sheet.frame_width) * (sheet.height / sheet.frame_height);
    check(
        cells_ok,
        char_file,
        "$.sheet.frameCount",
        "invalid sheet divisibility or frame count",
    );
    check(
        (0.0..=1.0).contains(&character.anchor.x) && (0.0..=1.0).contains(&character.anchor.y),
        char_file,
        "$.anchor",
        "anchor must be 0..1",
    );
    let is_looping = |name: &str| character.animations.get(name).is_some_and(|a| a.looping);
    if let Some(follow) = &project.cursor_follow {
        check(
            follow.priority.is_none_or(|priority| priority <= 100),
            file,
            "$.cursorFollow.priority",
            "priority must be 0..100",
        );
        check(
            (0.0..=2000.0).contains(&follow.stop_distance_dip),
            file,
            "$.cursorFollow.stopDistanceDip",
            "stop distance must be 0..2000 DIP",
        );
        check(
            (1..=8).contains(&follow.bands.len())
                && follow
                    .bands
                    .first()
                    .is_some_and(|band| band.min_distance_dip == 0.0)
                && follow
                    .bands
                    .windows(2)
                    .all(|pair| pair[0].min_distance_dip < pair[1].min_distance_dip),
            file,
            "$.cursorFollow.bands",
            "use 1..8 bands starting at zero, in strictly increasing distance order",
        );
        for (i, band) in follow.bands.iter().enumerate() {
            check(
                (0.0..=10000.0).contains(&band.min_distance_dip)
                    && (1.0..=1000.0).contains(&band.speed_dip_per_second)
                    && band.animation.as_deref().is_none_or(is_looping),
                file,
                &format!("$.cursorFollow.bands[{i}]"),
                "distance must be 0..10000 DIP, speed 1..1000 DIP/s, animation must loop",
            );
        }
    }
    check(
        is_looping(&character.default_animation),
        char_file,
        "$.defaultAnimation",
        "defaultAnimation must reference an existing looping animation",
    );
    if let Some(idle) = &project.idle {
        check(
            idle.directions
                .values()
                .all(|pose| pose.animation.as_deref().is_none_or(is_looping)),
            file,
            "$.idle.directions",
            "idle directions must reference existing looping animations",
        );
    }
    check(
        character.animations.len() <= 128,
        char_file,
        "$.animations",
        "at most 128 animations",
    );
    check(
        character.resources.len() <= 32
            && character
                .resources
                .keys()
                .all(|id| safe_id(id) && id != "sheet"),
        char_file,
        "$.resources",
        "maximum 32 resources; unique portable IDs other than sheet",
    );
    check(
        character.sounds.len() <= 32 && character.sounds.keys().all(|id| safe_id(id)),
        char_file,
        "$.sounds",
        "maximum 32 sounds with portable IDs",
    );
    for (name, animation) in &character.animations {
        let path = format!("$.animations.{name}");
        check(safe_id(name), char_file, &path, "invalid animation name");
        check(
            (1..=30).contains(&animation.fps),
            char_file,
            &path,
            "fps must be 1..30",
        );
        check(
            animation.scale_percent.is_none_or(|scale| {
                (MIN_ANIMATION_SCALE_PERCENT..=MAX_ANIMATION_SCALE_PERCENT).contains(&scale)
            }),
            char_file,
            &format!("{path}.scalePercent"),
            "scalePercent must be 50..200",
        );
        check(
            animation.anchor_x.is_none_or(|x| (0.0..=1.0).contains(&x)),
            char_file,
            &format!("{path}.anchorX"),
            "anchorX must be 0..1",
        );
        check(
            animation.frames.len() <= 256 && animation.regions.len() <= 256
                && (animation.frames.is_empty() || animation.regions.is_empty())
                && if let Some(id) = &animation.resource { character.resources.contains_key(id) }
                   else { (!animation.frames.is_empty() || !animation.regions.is_empty()) && animation.frames.iter().all(|f| *f < sheet.frame_count) },
            char_file,
            &path,
            "use existing resource, or legacy frames inside frameCount; frames and regions are exclusive (maximum 256)",
        );
    }
    let mut ids = HashSet::new();
    let mut sequences = Vec::new();
    let mut sequence_alternatives = Vec::new();
    let mut patterns = HashSet::new();
    let mut bands = Vec::new();
    let mut cursor_count = 0;
    let mut sequence_count = 0;
    for (index, binding) in project.bindings.iter().enumerate() {
        let path = format!("$.bindings[{index}]");
        check(
            safe_id(&binding.id) && ids.insert(&binding.id),
            file,
            &format!("{path}.id"),
            "binding IDs must be portable and unique",
        );
        let compatible = matches!(
            (&binding.trigger, &binding.effect),
            (
                Trigger::KeyDown { .. }
                    | Trigger::KeyUp { .. }
                    | Trigger::MouseDown { .. }
                    | Trigger::MouseUp { .. }
                    | Trigger::BuddyClick
                    | Trigger::Wheel { .. }
                    | Trigger::KeySequence { .. },
                Effect::PlayAnimation { .. }
            ) | (Trigger::WhileKeyDown { .. }, Effect::Move { .. })
                | (
                    Trigger::TypingRate { .. } | Trigger::WhileKeyDown { .. } | Trigger::Drag,
                    Effect::SustainAnimation { .. }
                )
                | (Trigger::CursorMove, Effect::FaceCursor { .. })
        );
        check(
            compatible,
            file,
            &path,
            "trigger/effect combination is not allowed",
        );
        if let Some(sound) = &binding.sound {
            check(
                !matches!(binding.trigger, Trigger::CursorMove)
                    && character.sounds.contains_key(&sound.resource)
                    && (0.0..=1.0).contains(&sound.volume),
                file,
                &format!("{path}.sound"),
                "sound requires a reaction, an existing audio resource, and volume 0..1",
            );
        }
        let mut compiled = None;
        let mut compiled_alternatives = Vec::new();
        match &binding.trigger {
            Trigger::KeyDown { alternatives, .. }
            | Trigger::KeyUp { alternatives, .. }
            | Trigger::WhileKeyDown { alternatives, .. } => {
                let held = matches!(binding.trigger, Trigger::WhileKeyDown { .. });
                check(
                    alternatives.len() < 32,
                    file,
                    &format!("{path}.trigger.alternatives"),
                    "maximum 32 key conditions per event",
                );
                for (index, (code, modifiers)) in binding.trigger.key_conditions().enumerate() {
                    let key_path = if index == 0 {
                        format!("{path}.trigger")
                    } else {
                        format!("{path}.trigger.alternatives[{}]", index - 1)
                    };
                    check(
                        is_supported_key(code) || (!held && code == "*"),
                        file,
                        &format!("{key_path}.code"),
                        if held {
                            "held key must be a supported physical key"
                        } else {
                            "unsupported physical key"
                        },
                    );
                    if let Some(modifiers) = modifiers {
                        check(
                            modifiers.iter().enumerate().all(|(index, modifier)| {
                                Some(*modifier) != KeyModifier::from_code(code)
                                    && !modifiers[..index].contains(modifier)
                            }),
                            file,
                            &format!("{key_path}.modifiers"),
                            "modifiers must be distinct and cannot include the main key's modifier",
                        );
                    }
                }
            }
            Trigger::TypingRate { min_kps, max_kps } => {
                let max = max_kps.unwrap_or(f64::INFINITY);
                check(
                    (0.5..=1000.0).contains(min_kps)
                        && max > *min_kps
                        && max_kps.is_none_or(|m| m.is_finite() && m <= 1000.0),
                    file,
                    &format!("{path}.trigger"),
                    "invalid typing rate range",
                );
                bands.push((*min_kps, max));
            }
            Trigger::KeySequence {
                keyword,
                layout,
                steps,
                timeout_ms,
                alternatives,
            } => {
                sequence_count += 1 + alternatives.len();
                check(
                    (500..=10000).contains(timeout_ms),
                    file,
                    &format!("{path}.trigger.timeoutMs"),
                    "timeout must be 500..10000 ms",
                );
                let result = match (keyword, layout, steps) {
                    (Some(word), Some(layout), None) => compile_keyword(word, *layout),
                    (None, None, Some(steps)) => validate_steps(steps).map(|_| steps.clone()),
                    _ => Err("provide keyword+layout or steps, exclusively".into()),
                };
                let results = std::iter::once(result).chain(
                    alternatives
                        .iter()
                        .map(|alias| compile_keyword(&alias.keyword, alias.layout)),
                );
                for (index, result) in results.enumerate() {
                    let keyword_path = if index == 0 {
                        format!("{path}.trigger")
                    } else {
                        format!("{path}.trigger.alternatives[{}]", index - 1)
                    };
                    match result {
                        Ok(steps) => {
                            let key = serde_json::to_string(&steps).expect("serializable steps");
                            check(
                                patterns.insert(key),
                                file,
                                &keyword_path,
                                "duplicate compiled key sequence",
                            );
                            if index == 0 {
                                compiled = Some(steps);
                            } else {
                                compiled_alternatives.push(steps);
                            }
                        }
                        Err(message) => check(false, file, &keyword_path, &message),
                    }
                }
            }
            Trigger::CursorMove => cursor_count += 1,
            _ => {}
        }
        match &binding.effect {
            Effect::PlayAnimation {
                animation,
                priority,
                cooldown_ms,
                duration_ms,
                stop_on_release,
                ..
            } => {
                check(
                    character
                        .animations
                        .get(animation)
                        .is_some_and(|a| !a.looping),
                    file,
                    &format!("{path}.effect.animation"),
                    "one-shot must reference an existing non-loop animation",
                );
                check(
                    *priority <= 100 && *cooldown_ms <= 10000
                        && duration_ms.is_none_or(|d| (1..=60000).contains(&d))
                        && (!*stop_on_release || matches!(binding.trigger, Trigger::KeyDown { .. } | Trigger::MouseDown { .. })),
                    file,
                    &format!("{path}.effect"),
                    "priority 0..100, cooldown 0..10000, duration 1..60000 ms; stopOnRelease requires keyDown/mouseDown",
                );
            }
            Effect::SustainAnimation {
                animation,
                priority,
            } => {
                check(
                    character
                        .animations
                        .get(animation)
                        .is_some_and(|a| a.looping),
                    file,
                    &format!("{path}.effect.animation"),
                    "sustain must reference an existing looping animation",
                );
                check(
                    priority.is_none_or(|value| value <= 100),
                    file,
                    &format!("{path}.effect.priority"),
                    "priority must be 0..100",
                );
            }
            Effect::Move {
                x,
                y,
                speed_dip_per_second,
                animation,
                priority,
            } => {
                check(
                    (-1.0..=1.0).contains(x)
                        && (-1.0..=1.0).contains(y)
                        && (*x != 0.0 || *y != 0.0)
                        && (1.0..=1000.0).contains(speed_dip_per_second),
                    file,
                    &format!("{path}.effect"),
                    "invalid movement vector or speed",
                );
                if let Some(name) = animation {
                    check(
                        name != "idle" && character.animations.get(name).is_some_and(|a| a.looping),
                        file,
                        &format!("{path}.effect.animation"),
                        "move must reference a non-idle looping animation",
                    );
                }
                check(
                    priority.is_none_or(|value| value <= 100),
                    file,
                    &format!("{path}.effect.priority"),
                    "priority must be 0..100",
                );
            }
            Effect::FaceCursor { directions } => {
                if let Some(directions) = directions {
                    for pose in directions.values() {
                        check(
                            pose.animation.as_deref().is_none_or(is_looping),
                            file,
                            &format!("{path}.effect.directions"),
                            "idle directions must reference existing looping animations",
                        );
                    }
                }
            }
        }
        sequences.push(compiled);
        sequence_alternatives.push(compiled_alternatives);
    }
    bands.sort_by(|a, b| a.0.total_cmp(&b.0));
    check(
        bands.len() <= 8 && bands.windows(2).all(|w| w[0].1 <= w[1].0),
        file,
        "$.bindings",
        "typing rate ranges must not overlap; maximum 8",
    );
    check(
        sequence_count <= 32 && cursor_count <= 1,
        file,
        "$.bindings",
        "maximum 32 sequences and 1 cursor binding",
    );
    if !errors.is_empty() {
        return Err(ValidationError(errors));
    }
    Ok(ValidatedProject {
        render_plan: legacy_plan(&character, "sheet"),
        project,
        character,
        sequences,
        sequence_alternatives,
    })
}

pub fn safe_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn portable_segment(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    !value.is_empty()
        && !value.ends_with(['.', ' '])
        && !value
            .chars()
            .any(|c| c.is_control() || r#"<>:"\|?*/"#.contains(c))
        && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, ValidationError> {
    if relative.is_empty()
        || relative.contains('\\')
        || Path::new(relative)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || relative.split('/').any(|part| !portable_segment(part))
    {
        return Err(problem(
            relative,
            "$",
            "UNSAFE_PATH",
            "expected portable relative path inside project",
        ));
    }
    let resolved = fs::canonicalize(root.join(relative))
        .map_err(|e| problem(relative, "$", "MISSING_ASSET", e.to_string()))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(problem(
            relative,
            "$",
            "UNSAFE_PATH",
            "asset escapes project root or is not a file",
        ));
    }
    Ok(resolved)
}

fn read_source(
    root: &Path,
    relative: &str,
    limit: u64,
    files: &mut BTreeMap<String, SourceAsset>,
) -> Result<Vec<u8>, ValidationError> {
    if let Some(existing) = files.get(relative) {
        return Ok(existing.bytes.clone());
    }
    let folded: String = relative.nfc().flat_map(char::to_lowercase).collect();
    if files
        .keys()
        .any(|name| name.nfc().flat_map(char::to_lowercase).collect::<String>() == folded)
    {
        return Err(problem(
            relative,
            "$",
            "PATH_COLLISION",
            "case or Unicode-normalized path collision",
        ));
    }
    let path = checked_path(root, relative)?;
    let before =
        fs::metadata(&path).map_err(|e| problem(relative, "$", "READ_FAILED", e.to_string()))?;
    if before.len() > limit {
        return Err(problem(
            relative,
            "$",
            "ASSET_TOO_LARGE",
            "asset exceeds size limit",
        ));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|e| problem(relative, "$", "READ_FAILED", e.to_string()))?;
    let after =
        fs::metadata(&path).map_err(|e| problem(relative, "$", "READ_FAILED", e.to_string()))?;
    if bytes.len() as u64 > limit
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != before.len()
    {
        return Err(problem(
            relative,
            "$",
            "PROJECT_CHANGED_DURING_SNAPSHOT",
            "source changed while being read",
        ));
    }
    files.insert(
        relative.into(),
        SourceAsset {
            path,
            modified: after.modified().ok(),
            hash: sha256(&bytes),
            bytes: bytes.clone(),
        },
    );
    if files.values().map(|file| file.bytes.len()).sum::<usize>() > ASSET_LIMIT {
        return Err(problem(
            relative,
            "$",
            "PROJECT_TOO_LARGE",
            "project assets exceed 64 MiB",
        ));
    }
    Ok(bytes)
}

fn check_png(bytes: &[u8], file: &str) -> Result<(u32, u32), ValidationError> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(problem(file, "$", "INVALID_PNG", "invalid PNG header"));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if !(1..=4096).contains(&width)
        || !(1..=4096).contains(&height)
        || u64::from(width) * u64::from(height) * 4 > 64 * 1024 * 1024
    {
        return Err(problem(
            file,
            "$",
            "IMAGE_TOO_LARGE",
            "PNG dimensions exceed decoded budget",
        ));
    }
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| problem(file, "$", "INVALID_PNG", e.to_string()))?;
    Ok((width, height))
}

pub fn load_project(path: impl AsRef<Path>) -> Result<LoadedProject, ValidationError> {
    load_project_inner(path.as_ref(), None)
}

/// Read the same source snapshot, using bundled frames if they are complete and current.
/// When prepared paths are returned, the frame bytes are not retained in render_assets.
pub fn load_project_with_prepared(
    path: impl AsRef<Path>,
    prepared: &Path,
) -> Result<LoadedProject, ValidationError> {
    load_project_inner(path.as_ref(), Some(prepared))
}

fn load_project_inner(
    input: &Path,
    prepared: Option<&Path>,
) -> Result<LoadedProject, ValidationError> {
    let entry = if input.is_dir() {
        input.join("buddy.project.json")
    } else {
        input.to_path_buf()
    };
    let root = fs::canonicalize(entry.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|e| problem("buddy.project.json", "$", "READ_FAILED", e.to_string()))?;
    let entry_name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            problem(
                "buddy.project.json",
                "$",
                "UNSAFE_PATH",
                "project filename must be UTF-8",
            )
        })?;
    let mut files = BTreeMap::new();
    let project_bytes = read_source(&root, entry_name, JSON_LIMIT, &mut files)?;
    let project: Project = parse_json(&project_bytes, entry_name)?;
    if project.schema_version != 1 {
        return Err(problem(
            entry_name,
            "$.schemaVersion",
            "UNSUPPORTED_SCHEMA",
            "only schemaVersion 1 is supported",
        ));
    }
    if Path::new(&project.character)
        .extension()
        .and_then(|s| s.to_str())
        != Some("json")
    {
        return Err(problem(
            entry_name,
            "$.character",
            "INVALID_ASSET_TYPE",
            "character manifest must be JSON",
        ));
    }
    let character_bytes = read_source(&root, &project.character, JSON_LIMIT, &mut files)?;
    let character: Character = parse_json(&character_bytes, &project.character)?;
    let character_dir = Path::new(&project.character)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    // Validate each manifest-relative path before joining; Path::join alone can
    // erase a prefix when given an absolute path.
    for name in std::iter::once(&character.image)
        .chain(std::iter::once(&character.license))
        .chain(character.resources.values().map(|r| &r.image))
        .chain(character.sounds.values().map(|sound| &sound.audio))
    {
        if Path::new(name)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
            || name.contains('\\')
        {
            return Err(problem(
                &project.character,
                "$",
                "UNSAFE_PATH",
                "manifest assets must be local relative paths",
            ));
        }
    }
    let image_relative = character_dir
        .join(&character.image)
        .to_string_lossy()
        .replace('\\', "/");
    let license_relative = character_dir
        .join(&character.license)
        .to_string_lossy()
        .replace('\\', "/");
    for name in [&image_relative, &project.product.icon] {
        if Path::new(name).extension().and_then(|s| s.to_str()) != Some("png") {
            return Err(problem(
                name,
                "$",
                "INVALID_ASSET_TYPE",
                "images must be PNG",
            ));
        }
    }
    if Path::new(&license_relative)
        .extension()
        .and_then(|s| s.to_str())
        != Some("txt")
    {
        return Err(problem(
            &license_relative,
            "$",
            "INVALID_ASSET_TYPE",
            "license must be UTF-8 .txt",
        ));
    }
    let png = read_source(&root, &image_relative, PNG_LIMIT, &mut files)?;
    let dimensions = check_png(&png, &image_relative)?;
    if dimensions != (character.sheet.width, character.sheet.height) {
        return Err(problem(
            &project.character,
            "$.sheet",
            "IMAGE_DIMENSION_MISMATCH",
            "declared sheet differs from PNG dimensions",
        ));
    }
    let icon = read_source(&root, &project.product.icon, PNG_LIMIT, &mut files)?;
    let (width, height) = check_png(&icon, &project.product.icon)?;
    if width != height || !(512..=1024).contains(&width) {
        return Err(problem(
            &project.product.icon,
            "$",
            "INVALID_ICON",
            "icon must be a 512..1024 square PNG",
        ));
    }
    let license = read_source(&root, &license_relative, JSON_LIMIT, &mut files)?;
    std::str::from_utf8(&license).map_err(|_| {
        problem(
            &license_relative,
            "$",
            "INVALID_LICENSE",
            "license must be UTF-8",
        )
    })?;
    let mut data = validate_parts(project, character).map_err(|mut error| {
        for diagnostic in &mut error.0 {
            if diagnostic.file == "buddy.project.json" {
                diagnostic.file = entry_name.into();
            }
        }
        error
    })?;
    if let Some(dmg) = &data.project.build.macos_dmg {
        for (asset, guide) in [
            (dmg.background.as_deref(), false),
            (dmg.guide.as_deref(), true),
        ] {
            let Some(path) = asset else { continue };
            let ext = Path::new(path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let bytes = read_source(&root, path, PNG_LIMIT, &mut files)?;
            let valid = match ext.as_str() {
                "png" => {
                    check_png(&bytes, path)?;
                    true
                }
                "jpg" | "jpeg" => {
                    matches!(image::guess_format(&bytes), Ok(image::ImageFormat::Jpeg))
                        && crate::probe_media(&bytes, None).is_ok()
                }
                "html" if guide => !bytes.is_empty() && std::str::from_utf8(&bytes).is_ok(),
                "pdf" if guide => {
                    bytes.starts_with(b"%PDF-")
                        && bytes[bytes.len().saturating_sub(1024)..]
                            .windows(5)
                            .any(|bytes| bytes == b"%%EOF")
                }
                _ => false,
            };
            if !valid {
                return Err(problem(
                    path,
                    "$",
                    "INVALID_INSTALL_ASSET",
                    if guide {
                        "expected a PDF, PNG, JPEG or UTF-8 HTML guide"
                    } else {
                        "expected a PNG or JPEG background"
                    },
                ));
            }
        }
    }
    let mut resource_paths = BTreeMap::new();
    for (id, resource) in &data.character.resources {
        let relative = character_dir
            .join(&resource.image)
            .to_string_lossy()
            .replace('\\', "/");
        let ext = Path::new(&relative)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&ext.as_str()) {
            return Err(problem(
                &relative,
                "$",
                "INVALID_ASSET_TYPE",
                "supported: PNG, JPEG, GIF, WebP, BMP",
            ));
        }
        read_source(&root, &relative, PNG_LIMIT, &mut files)?;
        resource_paths.insert(id.as_str(), relative);
    }
    let mut sound_files = BTreeMap::new();
    for (id, sound) in &data.character.sounds {
        let relative = character_dir
            .join(&sound.audio)
            .to_string_lossy()
            .replace('\\', "/");
        let extension = Path::new(&relative)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = read_source(&root, &relative, PNG_LIMIT, &mut files)?;
        validate_sound(&extension, &bytes)
            .map_err(|e| problem(&relative, "$", "INVALID_AUDIO", e))?;
        sound_files.insert(id.clone(), relative);
    }
    let mut hash = Sha256::new();
    for (name, file) in &files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((file.bytes.len() as u64).to_le_bytes());
        hash.update(&file.bytes);
    }
    let content_hash = format!("{:x}", hash.finalize());
    let mut render_assets = BTreeMap::new();
    let (footprints, prepared_render_paths) = if let Some((plan, footprints, paths)) = prepared
        .and_then(|directory| {
            crate::prepared::read_prepared_rendering(directory, &content_hash, &data.character)
        }) {
        data.render_plan = plan;
        (footprints, paths)
    } else {
        let mut budget = 0;
        let base = decode_media(&png, &mut budget, &mut render_assets)
            .map_err(|e| problem(&image_relative, "$", "INVALID_IMAGE", e))?;
        data.render_plan = legacy_plan(&data.character, &base.frames[0].image);
        let mut decoded = BTreeMap::new();
        for (id, relative) in resource_paths {
            let media = decode_media(&files[&relative].bytes, &mut budget, &mut render_assets)
                .map_err(|e| problem(&relative, "$", "INVALID_IMAGE", e))?;
            decoded.insert(id, media);
        }
        for (name, animation) in &data.character.animations {
            let media = if let Some(id) = &animation.resource {
                &decoded[id.as_str()]
            } else if !animation.regions.is_empty() {
                &base
            } else {
                continue;
            };
            let clip = compile_animation(animation, media).map_err(|e| {
                problem(
                    &data.project.character,
                    &format!("$.animations.{name}"),
                    "INVALID_ANIMATION",
                    e,
                )
            })?;
            data.render_plan.animations.insert(name.clone(), clip);
        }
        (
            crate::render_footprints(
                &data.character,
                &data.render_plan,
                std::iter::once(&base).chain(decoded.values()),
                &render_assets,
            ),
            BTreeMap::new(),
        )
    };
    Ok(LoadedProject {
        root,
        entry_relative_path: entry_name.into(),
        data,
        files,
        image_relative_path: image_relative,
        content_hash,
        render_assets,
        prepared_render_paths,
        sound_files,
        footprints,
    })
}
