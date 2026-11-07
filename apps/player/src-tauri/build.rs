fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_bootstrap",
            "subscribe_character_state",
            "ack_character_frame",
            "get_runtime_status",
            "reset_performance_session",
            "export_runtime_diagnostics",
            "request_input_access",
            "apply_user_preferences",
            "set_runtime_mode",
            "reset_user_preferences",
            "update_character_edit",
            "finish_character_edit",
            "cancel_character_edit",
        ]),
    ))
    .expect("Tauri build configuration");
}
