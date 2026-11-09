fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_bootstrap",
            "read_reaction_audio",
            "subscribe_character_state",
            "ack_character_frame",
            "set_character_hit_mask",
            "get_runtime_status",
            "set_launch_at_login",
            "reset_performance_session",
            "export_runtime_diagnostics",
            "request_input_access",
            "apply_user_preferences",
            "reset_user_preferences",
            "begin_character_drag",
            "update_character_edit",
            "finish_character_edit",
            "cancel_character_edit",
            "character_click",
            "show_buddy_menu",
        ]),
    ))
    .expect("Tauri build configuration");
}
