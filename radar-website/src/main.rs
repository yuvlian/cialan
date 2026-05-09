use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use radar_dumper::{Config, Overview};
use radar_reader::{CS2Reader, Player};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

#[derive(Serialize, Deserialize, Clone)]
struct WebState {
    map_name: String,
    overview: Option<Overview>,
    players: Vec<Player>,
}

struct AppState {
    tx: broadcast::Sender<WebState>,
    last_state: Arc<Mutex<WebState>>,
}

#[derive(Deserialize, Serialize, Clone)]
struct WebsiteConfig {
    pub host: String,
    pub port: u16,
    pub sleep_ms: u64,
}

impl Default for WebsiteConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            sleep_ms: 50,
        }
    }
}

impl WebsiteConfig {
    pub const PATH: &str = "radar-website.toml";
    pub fn load() -> Self {
        if Path::new(Self::PATH).exists() {
            let content = fs::read_to_string(Self::PATH).unwrap_or_default();
            toml::from_str(&content).unwrap_or_else(|_| Self::default())
        } else {
            let config = Self::default();
            let toml = toml::to_string_pretty(&config).unwrap();
            let _ = fs::write(Self::PATH, toml);
            config
        }
    }
}

fn main() {
    let (tx, _) = broadcast::channel(32);
    let last_state = Arc::new(Mutex::new(WebState {
        map_name: "Unknown".to_string(),
        overview: None,
        players: Vec::new(),
    }));

    let dumper_config = Config::load();
    let web_config = WebsiteConfig::load();
    let assets_dir = Path::new(&dumper_config.assets_dir);
    let radar_dir = assets_dir.join("radar");

    let state = Arc::new(AppState {
        tx: tx.clone(),
        last_state: last_state.clone(),
    });

    let last_state_clone = last_state.clone();
    let tx_clone = tx.clone();
    let radar_dir_clone = radar_dir.clone();
    thread::spawn(move || {
        let mut reader: Option<CS2Reader> = None;
        let mut overviews: HashMap<String, Overview> = HashMap::new();

        if radar_dir_clone.exists() {
            if let Ok(entries) = std::fs::read_dir(&radar_dir_clone) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("txt") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Some(ov) = Overview::parse(&content) {
                                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                                let map_key = stem.replace("_radar", "");
                                overviews.insert(map_key, ov);
                            }
                        }
                    }
                }
            }
        }
        println!("Loaded {} map overviews", overviews.len());

        loop {
            if reader.is_none() {
                reader = CS2Reader::new();
                if reader.is_none() {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }

            let r = reader.as_ref().unwrap();
            if !r.process.is_alive() {
                reader = None;
                let _ = tx_clone.send(WebState {
                    map_name: "Unknown".to_string(),
                    overview: None,
                    players: Vec::new(),
                });
                continue;
            }
            let mut map_name = r.get_map_name();
            if map_name.is_empty() || map_name == "<empty>" {
                map_name = "Unknown".to_string();
            }
            let players = r.get_players();

            let overview = overviews.get(&map_name).cloned();

            let full_state = WebState {
                map_name,
                overview,
                players,
            };

            {
                let mut last = last_state_clone.lock().unwrap();
                *last = full_state.clone();
            }

            let _ = tx_clone.send(full_state);

            thread::sleep(Duration::from_millis(web_config.sleep_ms));
        }
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .nest_service("/radar", ServeDir::new(radar_dir))
        .fallback_service(ServeDir::new("radar-website/page"))
        .with_state(state);

    let addr = format!("{}:{}", web_config.host, web_config.port);

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind(&addr).await.unwrap();
        println!("Listening on http://{}", addr);
        axum::serve(listener, app).await.unwrap();
    });
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();

    let initial = {
        let last = state.last_state.lock().unwrap();
        serde_json::to_string(&*last).unwrap()
    };

    if socket.send(Message::Text(initial.into())).await.is_err() {
        return;
    }

    while let Ok(new_state) = rx.recv().await {
        if let Ok(msg) = serde_json::to_string(&new_state) {
            if socket.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    }
}
