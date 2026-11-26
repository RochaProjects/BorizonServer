use std::{ffi::OsString, path::Path};

const HELP: &str = "Horizon dedicated engine bootstrap\n\nUsage:\n  horizon-engine --help\n  horizon-engine --version\n  horizon-engine --preflight\n\n--preflight checks eula.txt and loads/writes server.properties in the current directory.\nThe dedicated gameplay runtime is not connected yet; running without an option is intentionally refused.";

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Help,
    Version,
    Preflight,
    Run,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(arg) = args.next() else {
        return Ok(Command::Run);
    };
    if args.next().is_some() {
        return Err("unexpected extra argument; use --help".into());
    }
    match arg.to_str() {
        Some("--help" | "-h") => Ok(Command::Help),
        Some("--version" | "-V") => Ok(Command::Version),
        Some("--preflight") => Ok(Command::Preflight),
        Some(_) => Err("unknown argument; use --help".into()),
        None => Err("arguments must be valid UTF-8".into()),
    }
}

/// Run the bootstrap command in the current directory.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<String, String> {
    let command = parse_args(args)?;
    match command {
        Command::Help => Ok(HELP.into()),
        Command::Version => Ok(format!("horizon-engine {}", env!("CARGO_PKG_VERSION"))),
        Command::Preflight => {
            let dir = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
            preflight(&dir)
        }
        Command::Run => Err("the dedicated gameplay runtime has not been extracted yet; use --preflight to validate server files".into()),
    }
}

/// Validate the EULA and normalize the existing vanilla server.properties file.
/// This does not open a world, bind a socket, or start gameplay.
pub fn preflight(dir: &Path) -> Result<String, String> {
    let eula_path = dir.join("eula.txt");
    let agreed = horizon_server_files::eula::agreed(&eula_path).map_err(|error| {
        format!(
            "EULA not accepted or unreadable ({}): {error}",
            eula_path.display()
        )
    })?;
    if !agreed {
        return Err(format!(
            "EULA not accepted; review {} and set eula=true",
            eula_path.display()
        ));
    }

    let properties_path = dir.join("server.properties");
    let mut entropy = horizon_server_files::properties::SystemEntropy::default();
    let properties = horizon_server_files::properties::ServerProperties::from_file(
        &properties_path,
        &mut entropy,
    )?;
    properties
        .store(&properties_path)
        .map_err(|e| format!("write {}: {e}", properties_path.display()))?;

    Ok(format!(
        "Server preflight passed: level-name={}, server-port={}, properties={}",
        properties.level_name,
        properties.server_port,
        properties_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_args, preflight, run};
    use std::{ffi::OsString, path::PathBuf};

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("horizon-engine-{name}-{}", std::process::id()))
    }

    fn clean_dir(name: &str) -> PathBuf {
        let dir = scratch(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_help_version_and_preflight_without_accepting_unknown_options() {
        assert_eq!(parse_args([OsString::from("--help")]), Ok(Command::Help));
        assert_eq!(
            parse_args([OsString::from("--version")]),
            Ok(Command::Version)
        );
        assert_eq!(
            parse_args([OsString::from("--preflight")]),
            Ok(Command::Preflight)
        );
        assert!(parse_args([OsString::from("--dedicated")]).is_err());
        assert!(parse_args([OsString::from("--help"), OsString::from("extra")]).is_err());
    }

    #[test]
    fn help_and_version_run_without_opening_a_world() {
        assert!(
            run([OsString::from("--help")])
                .unwrap()
                .contains("--preflight")
        );
        assert!(
            run([OsString::from("--version")])
                .unwrap()
                .starts_with("horizon-engine ")
        );
    }

    #[test]
    fn default_execution_is_refused_until_gameplay_is_connected() {
        let error = run(std::iter::empty()).unwrap_err();
        assert!(error.contains("runtime has not been extracted"));
    }

    #[test]
    fn preflight_creates_default_eula_and_refuses_to_continue() {
        let dir = clean_dir("eula");
        let error = preflight(&dir).unwrap_err();
        assert!(error.contains("EULA not accepted"));
        let contents = std::fs::read_to_string(dir.join("eula.txt")).unwrap();
        assert!(contents.contains("eula=false"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_accepts_eula_and_normalizes_server_properties() {
        let dir = clean_dir("properties");
        std::fs::write(
            dir.join("eula.txt"),
            horizon_server_files::eula::file(0, true),
        )
        .unwrap();
        std::fs::write(dir.join("server.properties"), "server-port=25570\n").unwrap();

        let report = preflight(&dir).unwrap();
        assert!(report.contains("server-port=25570"));
        let properties = std::fs::read_to_string(dir.join("server.properties")).unwrap();
        assert!(properties.contains("server-port=25570"));
        assert!(properties.contains("level-name=world"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
