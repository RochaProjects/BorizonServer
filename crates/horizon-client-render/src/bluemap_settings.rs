//! Player-owned BlueMap preferences, independent of the tile downloader and renderer.

/// Where the player names the servers' maps, beside `horizon-graphics.txt`.
pub const SERVERS_FILE: &str = "horizon-bluemap.txt";
/// Default radius used when the configured value is zero.
pub const DEFAULT_RADIUS: u32 = 64;

/// Player-owned BlueMap settings: radius/auto-detect options and per-server map URLs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// How far around the player tiles are downloaded, in chunks; 0 uses `DEFAULT_RADIUS`.
    pub radius: u32,
    /// Look for the map of a server no `blueMap:` line names.
    pub auto_detect: bool,
    pub servers: Vec<Server>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub address: String,
    pub url: String,
    /// `(level, map id)`: which of the server's maps draws which level.
    pub maps: Vec<(String, String)>,
}

impl Settings {
    /// Parse `blueMapRadius:` as chunks or as blocks with a `b` suffix.
    pub fn parse_radius(value: &str) -> Option<u32> {
        let value = value.trim().to_ascii_lowercase();
        let blocks = value
            .strip_suffix("blocks")
            .or_else(|| value.strip_suffix('b'));
        let chunks = match blocks {
            Some(blocks) => blocks.trim().parse::<u32>().ok()?.div_ceil(16),
            None => value
                .strip_suffix("chunks")
                .unwrap_or(&value)
                .trim()
                .parse()
                .ok()?,
        };
        Some(chunks.min(4096))
    }

    /// Parse a `blueMap:` line's value.
    pub fn parse_server(value: &str) -> Option<Server> {
        let mut words = value.split_whitespace();
        let address = words.next()?.to_owned();
        let url = words.next()?.to_owned();
        let maps = words
            .filter_map(|word| word.split_once('='))
            .map(|(level, map)| (level_key(level), map.to_owned()))
            .collect();
        Some(Server { address, url, maps })
    }

    /// Parse `blueMap:` lines from the per-player server map file.
    pub fn parse_servers(text: &str) -> Vec<Server> {
        text.trim_start_matches('\u{feff}')
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(key, _)| key.trim() == "blueMap")
            .filter_map(|(_, value)| Self::parse_server(value))
            .collect()
    }

    /// Serialize the options stored in `horizon-graphics.txt`.
    pub fn serialize(&self) -> String {
        format!(
            "blueMapRadius:{}\nblueMapAutoDetect:{}\n",
            self.radius, self.auto_detect
        )
    }

    /// Find the settings for a server address using the stable server-folder key.
    pub fn server(&self, address: &str) -> Option<&Server> {
        let folder = horizon_sim_data::server_address::server_folder(address);
        self.servers.iter().find(|server| {
            horizon_sim_data::server_address::server_folder(&server.address) == folder
        })
    }

    pub fn radius_blocks(&self) -> i32 {
        self.radius_chunks() as i32 * 16
    }

    pub fn radius_chunks(&self) -> u32 {
        if self.radius == 0 {
            DEFAULT_RADIUS
        } else {
            self.radius
        }
    }
}

fn level_key(level: &str) -> String {
    match level {
        "nether" => "minecraft:the_nether".into(),
        "end" => "minecraft:the_end".into(),
        _ if level.contains(':') => level.into(),
        _ => format!("minecraft:{level}"),
    }
}
