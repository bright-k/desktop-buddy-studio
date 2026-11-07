use tauri::Url;

pub fn allow_player_navigation(url: &Url) -> bool {
    let (scheme, host) = if cfg!(windows) {
        ("http", "tauri.localhost")
    } else {
        ("tauri", "localhost")
    };
    is_entry_document(url, scheme, host)
}

fn is_entry_document(url: &Url, scheme: &str, host: &str) -> bool {
    url.scheme() == scheme
        && url.host_str() == Some(host)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "" | "/" | "/index.html")
        && url.query().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_origins_accept_only_the_bundled_entry_and_its_settings_fragment() {
        for (scheme, host) in [("tauri", "localhost"), ("http", "tauri.localhost")] {
            for suffix in ["", "/", "/index.html", "/index.html#settings"] {
                assert!(is_entry_document(
                    &Url::parse(&format!("{scheme}://{host}{suffix}")).unwrap(),
                    scheme,
                    host
                ));
            }
        }
    }
    #[test]
    fn remote_file_data_lookalike_and_other_local_documents_are_rejected() {
        for url in [
            "https://example.com/index.html",
            "file:///etc/passwd",
            "about:blank",
            "data:text/html,hello",
            "javascript:alert(1)",
            "http://tauri.localhost.evil.test/index.html",
            "http://tauri.localhost:1234/index.html",
            "http://user@tauri.localhost/index.html",
            "http://tauri.localhost/index.html?redirect=external",
            "http://tauri.localhost/other.html",
            "http://tauri.localhost/%2e%2e/etc/passwd",
            "tauri://remote/index.html",
            "tauri://localhost:1234/index.html",
            "tauri://user@localhost/index.html",
            "tauri://localhost/other.html",
            "tauri://localhost/index.html?external=true",
        ] {
            let url = Url::parse(url).unwrap();
            assert!(!is_entry_document(&url, "tauri", "localhost"), "{url}");
            assert!(!is_entry_document(&url, "http", "tauri.localhost"), "{url}");
        }
        assert!(!is_entry_document(
            &Url::parse("http://tauri.localhost/index.html").unwrap(),
            "tauri",
            "localhost"
        ));
        assert!(!is_entry_document(
            &Url::parse("tauri://localhost/index.html").unwrap(),
            "http",
            "tauri.localhost"
        ));
    }
}
