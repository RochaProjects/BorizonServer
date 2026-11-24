//! Dedicated-server command-line and configuration bootstrap.
use horizon_server_files::{
    eula,
    properties::{ServerProperties, SystemEntropy},
};
use std::{error::Error, path::PathBuf, time::Instant};

/// Immutable result of the server-only preflight, consumed by the live engine
/// after its vanilla resources are loaded.
pub struct ServerSetup {
    pub props: ServerProperties,
    pub universe: PathBuf,
    pub level_name: String,
    pub port: Option<u16>,
    pub started: Instant,
}

/// Read the dedicated server's arguments, `server.properties`, and EULA.
/// Returns `None` for `--help`, `--initSettings`, or an unaccepted EULA; these
/// are normal early exits, not runtime failures. Logging remains supplied by
/// the caller so the server's established log format and output are retained.
pub fn preflight(
    args: &[String],
    mut write_log: impl FnMut(&str, &str),
) -> Result<Option<ServerSetup>, Box<dyn Error>> {
    let mut universe = PathBuf::from(".");
    let mut world = None;
    let mut port = None;
    let mut init_settings = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let name = arg.trim_start_matches('-');
        match name {
            "nogui"
            | "demo"
            | "bonusChest"
            | "forceUpgrade"
            | "eraseCache"
            | "recreateRegionFiles"
            | "safeMode"
            | "jfrProfile" => {}
            "initSettings" => init_settings = true,
            "help" => {
                eprintln!(
                    "Option                Description\n------                -----------\n--help\n--initSettings        Initializes 'server.properties' and 'eula.txt', then quits\n--nogui\n--port <Integer>      (default: -1)\n--universe <String>   (default: .)\n--world <String>"
                );
                return Ok(None);
            }
            "universe" => universe = PathBuf::from(it.next().ok_or("--universe needs a value")?),
            "world" => world = Some(it.next().ok_or("--world needs a value")?.clone()),
            "port" => {
                let value: i32 = it.next().ok_or("--port needs a value")?.parse()?;
                port = u16::try_from(value).ok();
            }
            "serverId" | "pidFile" => {
                let value = it.next().ok_or("missing value")?;
                if name == "pidFile" {
                    std::fs::write(value, std::process::id().to_string())?;
                }
            }
            _ => return Err(format!("{arg} is not a recognized option").into()),
        }
    }
    let started = Instant::now();
    let settings = PathBuf::from("server.properties");
    let props = ServerProperties::from_file(&settings, &mut SystemEntropy::default())?;
    for warning in &props.warnings {
        write_log("WARN", warning);
    }
    props.store(&settings)?;
    // `RegionFileVersion.configure`: this changes how every region file the
    // server writes is compressed; preserve the default if the name is invalid.
    if !horizon_world::compression::Compression::configure(&props.region_file_compression) {
        write_log(
            "ERROR",
            &format!(
                "Invalid `region-file-compression` value `{}` in server.properties. Please use one of: {}",
                props.region_file_compression,
                horizon_world::compression::REGION_FILE_NAMES.join(",")
            ),
        );
    }
    let eula_file = PathBuf::from("eula.txt");
    let agreed = match eula::agreed(&eula_file) {
        Ok(agreed) => agreed,
        Err(warning) => {
            write_log("WARN", &warning);
            false
        }
    };
    if init_settings {
        let abs =
            |path: &PathBuf| std::env::current_dir().map_or(path.clone(), |dir| dir.join(path));
        write_log(
            "INFO",
            &format!(
                "Initialized '{}' and '{}'",
                abs(&settings).display(),
                abs(&eula_file).display()
            ),
        );
        return Ok(None);
    }
    if !agreed {
        write_log(
            "INFO",
            "You need to agree to the EULA in order to run the server. Go to eula.txt for more info.",
        );
        return Ok(None);
    }
    let level_name = world.unwrap_or_else(|| props.level_name.clone());
    Ok(Some(ServerSetup {
        props,
        universe,
        level_name,
        port,
        started,
    }))
}

#[cfg(test)]
mod tests {
    use super::preflight;

    #[test]
    fn help_exits_before_reading_or_writing_server_files() {
        let result = preflight(&["--help".into()], |_, _| {});
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn unknown_argument_fails_before_reading_server_files() {
        let error = match preflight(&["--not-a-server-option".into()], |_, _| {}) {
            Err(error) => error,
            Ok(_) => panic!("unknown options must remain errors"),
        };
        assert!(error.to_string().contains("not a recognized option"));
    }
}
