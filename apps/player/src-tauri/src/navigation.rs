use tauri::Url;

pub fn allow_player_navigation(url: &Url) -> bool {
    let (scheme, host) = player_origin();
    is_entry_document(url, scheme, host) && url.query().is_none()
}

fn player_origin() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("http", "tauri.localhost")
    } else {
        ("tauri", "localhost")
    }
}

pub fn overlay_recovery_url(generation: u32) -> Result<Url, String> {
    let (scheme, host) = player_origin();
    Url::parse(&format!(
        "{scheme}://{host}/index.html?overlay-recovery={generation}"
    ))
    .map_err(|error| error.to_string())
}

pub fn allow_overlay_navigation(url: &Url) -> bool {
    let (scheme, host) = player_origin();
    is_entry_document(url, scheme, host)
        && url.query().is_none_or(|query| {
            url.path() == "/index.html"
                && url.fragment().is_none()
                && query
                    .strip_prefix("overlay-recovery=")
                    .is_some_and(|generation| {
                        !generation.is_empty()
                            && generation.bytes().all(|b| b.is_ascii_digit())
                            && generation.parse::<u32>().is_ok_and(|value| value > 0)
                    })
        })
}

fn is_entry_document(url: &Url, scheme: &str, host: &str) -> bool {
    url.scheme() == scheme
        && url.host_str() == Some(host)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "" | "/" | "/index.html")
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
            assert!(!allow_player_navigation(&url), "{url}");
            assert!(!allow_overlay_navigation(&url), "{url}");
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

    #[test]
    fn only_the_overlay_accepts_its_numeric_recovery_url() {
        let url = overlay_recovery_url(7).unwrap();
        assert!(allow_overlay_navigation(&url));
        assert!(!allow_player_navigation(&url));
        let mut invalid = url;
        invalid.set_query(Some("overlay-recovery=7&redirect=external"));
        assert!(!allow_overlay_navigation(&invalid));
        invalid.set_query(Some("overlay-recovery=0"));
        assert!(!allow_overlay_navigation(&invalid));
    }
}
