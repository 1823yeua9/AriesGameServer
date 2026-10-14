use crate::{Config, Platform, rms_topic};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{
        FromRequestParts, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    net::{Ipv4Addr, TcpListener},
    sync::Arc,
    time::Duration,
};

fn reply(status: u16, payload: Value) -> Response {
    let mut response = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(payload),
    )
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

async fn http_handler(State(platform): State<Arc<Platform>>, request: Request<Body>) -> Response {
    let (mut parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let method = parts.method.to_string();
    let public =
        path.starts_with("/.a.pvp.net/") || (method == "GET" && path.starts_with("/v1/config/"));
    let authorized = platform.authorized(
        parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        false,
    );
    if !public && !authorized {
        let mut response = reply(401, json!({"error":"unauthorized"}));
        response.headers_mut().insert(
            "www-authenticate",
            "Basic realm=\"RiotRemoting\"".parse().unwrap(),
        );
        return response;
    }
    if parts
        .headers
        .get("upgrade")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
    {
        // Upgrades require local credentials even on a decorated public route.
        if !authorized {
            return reply(401, json!({"error":"unauthorized"}));
        }
        let protocols = parts
            .headers
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let binary = protocols.trim().is_empty();
        if !binary
            && !protocols
                .split(',')
                .any(|p| p.trim() == "WAMP-1.0-RiotRemoting")
        {
            return reply(400, json!({"error":"bad_upgrade"}));
        }
        let Ok(upgrade) = WebSocketUpgrade::from_request_parts(&mut parts, &platform).await else {
            return reply(400, json!({"error":"bad_upgrade"}));
        };
        let upgrade = upgrade
            .max_message_size(1024 * 1024)
            .max_frame_size(1024 * 1024);
        let upgrade = if binary {
            upgrade
        } else {
            upgrade.protocols(["WAMP-1.0-RiotRemoting"])
        };
        return upgrade.on_upgrade(move |socket| websocket(socket, platform, binary));
    }
    let Ok(body) = to_bytes(body, 1024 * 1024).await else {
        return reply(413, json!({"error":"body_too_large"}));
    };
    if method == "GET"
        && (path.starts_with("/v1/config/") || path.starts_with("/.a.pvp.net/v1/config/"))
        && let Some(gate) = &platform.config.startup_gate
    {
        if gate.exists() {
            let waiting = gate.with_extension("waiting");
            if tokio::fs::write(waiting, b"configuration request held")
                .await
                .is_err()
            {
                return reply(500, json!({"error":"startup_gate_marker_failed"}));
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        while gate.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if gate.exists() {
            return reply(503, json!({"error":"startup_gate_timeout"}));
        }
    }
    let target = parts.uri.to_string();
    // Persistence can touch disk; keep it off the async I/O executor.
    let result = tokio::task::spawn_blocking(move || platform.route(&method, &target, &body)).await;
    match result {
        Ok((status, value)) => reply(status, value),
        Err(_) => reply(500, json!({"error":"internal_error"})),
    }
}

async fn admin_handler(State(platform): State<Arc<Platform>>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    if !platform.authorized(
        parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        true,
    ) {
        return reply(401, json!({"error":"unauthorized"}));
    }
    let Ok(body) = to_bytes(body, 4096).await else {
        return reply(413, json!({"error":"body_too_large"}));
    };
    let result = tokio::task::spawn_blocking(move || {
        platform.admin_route(parts.method.as_str(), parts.uri.path(), &body)
    })
    .await;
    match result {
        Ok((status, value)) => reply(status, value),
        Err(_) => reply(500, json!({"error":"internal_error"})),
    }
}

async fn send_message(socket: &mut WebSocket, value: &Value, binary: bool) -> bool {
    let message = if binary {
        Message::Binary(rmp_serde::to_vec_named(value).unwrap().into())
    } else {
        Message::Text(serde_json::to_string(value).unwrap().into())
    };
    socket.send(message).await.is_ok()
}
async fn websocket(mut socket: WebSocket, platform: Arc<Platform>, binary: bool) {
    let mut events = platform.events.subscribe();
    let mut topics = HashSet::<String>::new();
    if !send_message(
        &mut socket,
        &json!([0, uuid::Uuid::new_v4().to_string(), 1, "local-backend/0.1"]),
        binary,
    )
    .await
    {
        return;
    }
    loop {
        tokio::select! {
            event=events.recv()=> {
                match event {
                    Ok(event) if topics.contains(rms_topic)=>if !send_message(&mut socket,&event,binary).await {break;},
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{let _=socket.send(Message::Close(None)).await;break;},
                    Err(tokio::sync::broadcast::error::RecvError::Closed)=>break,
                    _=>()
                }
            }
            incoming=socket.recv()=> {
                let Some(Ok(incoming))=incoming else {break};
                let (message,reply_binary): (Value,bool)=match incoming {
                    Message::Binary(bytes)=>match rmp_serde::from_slice(&bytes) {Ok(v)=>(v,true),Err(_)=>continue},
                    Message::Text(text)=>match serde_json::from_str(text.as_str()) {Ok(v)=>(v,false),Err(_)=>continue},
                    Message::Ping(bytes)=>{if socket.send(Message::Pong(bytes)).await.is_err(){break;}continue;},
                    Message::Close(_)=>{let _=socket.send(Message::Close(None)).await;break;},
                    _=>continue
                };
                let Some(values)=message.as_array() else {continue};
                match values.first().and_then(Value::as_u64) {
                    Some(5)=>{if let Some(topic)=values.get(1).and_then(Value::as_str){topics.insert(topic.into());}},
                    Some(6)=>{if let Some(topic)=values.get(1).and_then(Value::as_str){topics.remove(topic);}},
                    Some(2) if values.len()>=3=>{
                        let response=match values[2].as_str() {
                            Some("riotclient.ping")=>json!([3,values[1],"pong"]),
                            Some("riotclient.app_port")=>json!([3,values[1],platform.config.port]),
                            _=>json!([4,values[1],"wamp.error.no_such_procedure",values[2]])
                        };
                        if !send_message(&mut socket,&response,reply_binary).await {break;}
                    },
                    _=>()
                }
            }
        }
    }
}

pub async fn serve(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tls_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))?;
    let admin_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.admin_port))?;
    let mut config = config;
    config.port = tls_listener.local_addr()?.port();
    config.admin_port = admin_listener.local_addr()?.port();
    let platform = Platform::new(config)?;
    serve_bound(platform, tls_listener, admin_listener).await
}

pub async fn serve_platform(
    platform: Arc<Platform>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if platform.config.port == 0 || platform.config.admin_port == 0 {
        return Err("serve_platform requires nonzero ports; use serve for assigned ports".into());
    }
    let tls_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, platform.config.port))?;
    let admin_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, platform.config.admin_port))?;
    serve_bound(platform, tls_listener, admin_listener).await
}

async fn serve_bound(
    platform: Arc<Platform>,
    tls_listener: TcpListener,
    admin_listener: TcpListener,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &platform.config.cert_path,
        &platform.config.key_path,
    )
    .await?;
    tls_listener.set_nonblocking(true)?;
    admin_listener.set_nonblocking(true)?;
    let admin_listener = tokio::net::TcpListener::from_std(admin_listener)?;
    let server = axum_server::from_tcp_rustls(tls_listener, tls)?;
    let config = &platform.config;
    std::fs::create_dir_all(&config.persistence_path)?;
    if let Some(parent) = config.state_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = json!({"host":"127.0.0.1","port":config.port,"auth_token":config.auth_token,"admin_host":"127.0.0.1","admin_port":config.admin_port,"admin_token":config.admin_token,"pid":std::process::id(),"service":"aries-platform"});
    let temporary_state = config.state_path.with_extension("ready.tmp");
    std::fs::write(&temporary_state, serde_json::to_vec_pretty(&state)?)?;
    std::fs::rename(&temporary_state, &config.state_path)?;
    println!(
        "Aries platform ready: HTTPS 127.0.0.1:{}, admin 127.0.0.1:{}",
        config.port, config.admin_port
    );
    let app = Router::new()
        .fallback(http_handler)
        .with_state(platform.clone());
    let admin = Router::new().fallback(admin_handler).with_state(platform);
    tokio::select! {
        result=server.serve(app.into_make_service())=>result?,
        result=axum::serve(admin_listener,admin)=>result?,
        _=tokio::signal::ctrl_c()=>()
    }
    Ok(())
}
