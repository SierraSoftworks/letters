//! `letters-server [--config <file>] [--check]`
//!
//! Reads its settings from the file named, or by `LETTERS_CONFIG`, or from `/etc/letters/server.toml`, and
//! serves until it is told to stop. `--check` reads the settings, says what is wrong with them and exits.

use std::process::ExitCode;
use std::sync::Arc;

use letters_server::config::Config;
use letters_server::store::Store;
use letters_server::{App, api, log};

const DEFAULT_CONFIG: &str = "/etc/letters/server.toml";

fn main() -> ExitCode {
    let usage = || {
        eprintln!("usage: letters-server [--config <file>] [--check]");
        ExitCode::from(2)
    };
    let mut args = std::env::args().skip(1);
    let (mut path, mut check) = (std::env::var("LETTERS_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG.into()), false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--config" => match args.next() {
                Some(file) => path = file,
                None => return usage(),
            },
            _ => return usage(),
        }
    }
    let config = std::fs::read_to_string(&path).map_err(|error| error.to_string());
    let config = match config.and_then(|text| Config::parse(&text).map_err(|error| error.to_string())) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("letters-server: {path}: {error}");
            return ExitCode::from(2);
        }
    };
    if check {
        println!("{path} is sound: {} spaces, signing in at {}", config.spaces.len(), config.auth.oidc.issuer);
        return ExitCode::SUCCESS;
    }
    match serve(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("letters-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open(&config.server.state)?;
    let listen = config.server.listen;
    let app = Arc::new(App::new(config, store));
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(listen).await?;
        log(&format!("listening on {listen} as {}", app.config.server.url));
        // The index is brought up to the folders while the first requests are already answered.
        let scanning = app.clone();
        tokio::task::spawn_blocking(move || scanning.scan());
        axum::serve(listener, api::router(app)).with_graceful_shutdown(stopped()).await?;
        Ok(())
    })
}

/// Resolves when the server is asked to stop: Ctrl-C, or the signal a service manager sends.
async fn stopped() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        if let Ok(mut terminate) = terminate {
            tokio::select! {
                _ = interrupt => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = interrupt.await;
}
