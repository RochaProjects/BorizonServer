use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match horizon_engine::run(env::args_os().skip(1)) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("horizon-engine: {error}");
            ExitCode::FAILURE
        }
    }
}
