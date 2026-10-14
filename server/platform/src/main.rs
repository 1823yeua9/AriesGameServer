use aries_platform::{Config, Platform, serve};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--help") || args.is_empty() {
        println!(
            "aries-platform --config <platform.json>\naries-platform --replay [--config <platform.json>] (JSON requests on stdin)"
        );
        return Ok(());
    }
    let mut config_path = None;
    let mut replay = false;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--config" if config_path.is_none() => {
                config_path = Some(arguments.next().ok_or("missing config path")?)
            }
            "--replay" if !replay => replay = true,
            _ => return Err(format!("unknown or duplicate argument: {argument}").into()),
        }
    }
    let config = if let Some(path) = config_path {
        serde_json::from_slice::<Config>(&std::fs::read(path)?)?
    } else if replay {
        Config::default()
    } else {
        return Err("--config is required when serving".into());
    };
    if replay {
        let mut platform = Platform::new(config.clone())?;
        for line in io::stdin().lock().lines() {
            let request: Value = serde_json::from_str(&line?)?;
            if request["reset"] == true {
                platform = Platform::new(config.clone())?;
            }
            if let Some(state) = request.get("session") {
                let mut session = platform.session.lock().unwrap();
                session.party_state = state["party_state"].as_str().unwrap().into();
                session.previous_state = state["party_previous_state"].as_str().unwrap().into();
                session.party_version = state["party_version"].as_u64().unwrap();
                session.loop_state = state["session_loop_state"].as_str().unwrap().into();
                session.version = state["session_version"].as_u64().unwrap();
                session.match_id = state["core_game_match_id"].as_str().unwrap().into();
                session.player_version = state["core_game_player_version"].as_u64().unwrap();
            }
            if let Some(state) = request.get("inventory") {
                let mut inventory = platform.inventory.lock().unwrap();
                inventory.version = state["version"].as_u64().unwrap();
                inventory.loadout = if state["loadout"].is_null() {
                    None
                } else {
                    Some(state["loadout"].clone())
                };
            }
            let body = if let Some(raw) = request["raw_body"].as_str() {
                raw.as_bytes().to_vec()
            } else {
                serde_json::to_vec(request.get("body").unwrap_or(&json!({})))?
            };
            let method = request["method"].as_str().unwrap_or("GET");
            let path = request["path"].as_str().unwrap_or("/");
            let (status, payload) = if path.starts_with("/dev/") {
                platform.admin_route(method, path, &body)
            } else {
                platform.route(method, path, &body)
            };
            println!("{}", json!({"status":status,"payload":payload}));
            io::stdout().flush()?;
        }
        Ok(())
    } else {
        serve(config).await
    }
}
