//! Shared end-to-end harness used by both the server stack suite and the client sign-in suite:
//! headless Chrome automation (`chrome`/`flow`), a Keycloak admin client (`keycloak`), and the
//! helpers that locate the running stack, its CA, and the assets tree.

pub mod chrome;
pub mod flow;
pub mod keycloak;

pub const REALM: &str = "rift";
pub const PASSWORD: &str = "e2e-password-1";

pub fn auth_base() -> String {
    std::env::var("RIFT_E2E_AUTH").unwrap_or_else(|_| "https://auth.rift.localhost".to_owned())
}

pub fn site_base() -> String {
    std::env::var("RIFT_E2E_SITE").unwrap_or_else(|_| "https://rift.localhost".to_owned())
}

pub fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_nanos()
    )
}

/// Polls for a file to appear and returns its contents, or `None` past the timeout.
pub fn wait_for_file(path: &std::path::Path, seconds: f32) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(seconds);
    while std::time::Instant::now() < deadline {
        if let Ok(contents) = std::fs::read_to_string(path)
            && !contents.is_empty()
        {
            return Some(contents);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

/// The reverse proxy's local CA, for the client's sign-in TLS. Honours `RIFT_CLIENT_EXTRA_CA`,
/// otherwise copies it out of the running stack's reverse-proxy container.
pub fn caddy_ca() -> String {
    if let Ok(path) = std::env::var("RIFT_CLIENT_EXTRA_CA") {
        return path;
    }
    let out = std::env::temp_dir().join("rift-caddy-root.crt");
    let compose = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docker/docker-compose.yaml"
    );
    let status = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            compose,
            "cp",
            "reverse-proxy:/data/caddy/pki/authorities/local/root.crt",
            out.to_str().expect("ca path"),
        ])
        .status()
        .expect("export caddy CA");
    assert!(status.success(), "exporting the caddy CA failed");
    out.to_string_lossy().into_owned()
}

/// The client side resolves content (areas, actor models) too, so a test process needs the assets
/// root. Point it at the repo's `assets/` unless the environment already set it.
pub fn ensure_assets() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("RIFT_ASSETS").is_none() {
            // SAFETY: runs before the client app starts any threads or loads content.
            unsafe {
                std::env::set_var(
                    "RIFT_ASSETS",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"),
                );
            }
        }
    });
}
