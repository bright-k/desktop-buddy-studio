mod bundle;
mod keys;
mod media;
mod sequence;
mod types;
mod validation;

pub use bundle::*;
pub use keys::*;
pub use media::*;
pub use sequence::*;
pub use types::*;
pub use validation::*;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u32 = 1;

pub fn project_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Project)).expect("schema is serializable")
}
pub fn character_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Character)).expect("schema is serializable")
}
