//! Stable, filesystem-safe name for a server address used by client-side caches.

/// Lowercase an address and make it safe as one path component. The default
/// Minecraft port is omitted (`play.example.net:25565` becomes
/// `play.example.net`).
pub fn server_folder(address: &str) -> String {
    let address = address.trim().to_ascii_lowercase();
    let address = address.strip_suffix(":25565").unwrap_or(&address);
    let name: String = address
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        "_".into()
    } else {
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use super::server_folder;

    #[test]
    fn address_becomes_stable_safe_folder_name() {
        assert_eq!(
            server_folder(" Play.Example.Net:25565 "),
            "play.example.net"
        );
        assert_eq!(
            server_folder("play.example.net:25566"),
            "play.example.net_25566"
        );
        assert_eq!(server_folder("..."), "_");
    }
}
