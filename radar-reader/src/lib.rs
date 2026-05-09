use memory::{Process, parse_signature};
use serde::{Deserialize, Serialize};
use std::ffi::CStr;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    pub health: i32,
    pub team: u8,
    pub pos: [f32; 3],
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Signatures {
    pub entity_list: String,
    pub global_vars: String,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
#[allow(non_snake_case)]
pub struct Offsets {
    pub m_hPawn: usize,
    pub m_iHealth: usize,
    pub m_iTeamNum: usize,
    pub m_pGameSceneNode: usize,
    pub m_vecAbsOrigin: usize,
    pub m_sSanitizedPlayerName: usize,
    pub m_map_name: usize,
    pub controller_entry: usize,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub signatures: Signatures,
    pub offsets: Offsets,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            signatures: Signatures {
                entity_list: "48 89 0D ? ? ? ? E9 ? ? ? ? CC".to_string(),
                global_vars: "48 89 15 ? ? ? ? 48 89 42".to_string(),
            },
            offsets: Offsets {
                m_hPawn: 0x6BC,
                m_iHealth: 0x34C,
                m_iTeamNum: 0x3EB,
                m_pGameSceneNode: 0x330,
                m_vecAbsOrigin: 0xC8,
                m_sSanitizedPlayerName: 0x6F4,
                m_map_name: 0x188,
                controller_entry: 0x70,
            },
        }
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn get_config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let path = "radar-reader.toml";
        if Path::new(path).exists() {
            let content = fs::read_to_string(path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_else(|_| Config::default())
        } else {
            let config = Config::default();
            let toml = toml::to_string_pretty(&config).unwrap();
            let _ = fs::write(path, toml);
            config
        }
    })
}

pub struct CS2Reader {
    pub process: Process,
    entity_list_ptr: usize,
    global_vars_ptr: usize,
    offsets: Offsets,
}

impl CS2Reader {
    pub fn new() -> Option<Self> {
        let config = get_config();
        let mut process = Process::new()?;
        if !process.attach_process("cs2.exe") {
            return None;
        }

        let client = process.get_module("client.dll");
        if client.base == 0 {
            return None;
        }

        let entity_list_sig = parse_signature(&config.signatures.entity_list);
        let entity_list_ptr = process.read_offset_from_module::<i32>(client, &entity_list_sig, 3);

        let global_vars_sig = parse_signature(&config.signatures.global_vars);
        let global_vars_ptr = process.read_offset_from_module::<i32>(client, &global_vars_sig, 3);

        if entity_list_ptr == 0 || global_vars_ptr == 0 {
            return None;
        }

        Some(Self {
            process,
            entity_list_ptr,
            global_vars_ptr,
            offsets: config.offsets,
        })
    }

    pub fn get_map_name(&self) -> String {
        let global_vars = self.process.read::<usize>(self.global_vars_ptr);
        if global_vars == 0 {
            return "Unknown".to_string();
        }

        let mut name_bytes = [0u8; 32];
        let name_ptr = self
            .process
            .read::<usize>(global_vars + self.offsets.m_map_name);
        if name_ptr != 0 && name_ptr > 0x1000000 {
            if self
                .process
                .read_raw(name_ptr, name_bytes.as_mut_ptr() as _, 64)
            {
                if let Ok(s) = CStr::from_bytes_until_nul(&name_bytes) {
                    let map = s.to_string_lossy();
                    let clean_map = map.split('/').last().unwrap_or(&map);
                    if !clean_map.is_empty() && clean_map != "<empty>" {
                        return clean_map.to_string();
                    }
                }
            }
        }
        "Unknown".to_string()
    }

    pub fn get_players(&self) -> Vec<Player> {
        let mut players = Vec::with_capacity(512);
        let entity_list = self.process.read::<usize>(self.entity_list_ptr);
        if entity_list == 0 {
            return players;
        }

        for i in 1..=players.capacity() {
            let list_entry = self
                .process
                .read::<usize>(entity_list + (8 * (i >> 9)) + 16);
            if list_entry == 0 {
                continue;
            }

            let controller = self
                .process
                .read::<usize>(list_entry + self.offsets.controller_entry * (i & 0x1FF));
            if controller == 0 || controller < 0x1000000 || controller > 0x7FFFFFFFFFFF {
                continue;
            }

            let pawn_handle = self.process.read::<u32>(controller + self.offsets.m_hPawn);
            if pawn_handle == 0 || pawn_handle == 0xFFFFFFFF {
                continue;
            }

            let pawn_list_entry = self
                .process
                .read::<usize>(entity_list + (8 * ((pawn_handle & 0x7FFF) as usize >> 9)) + 16);
            if pawn_list_entry == 0 {
                continue;
            }

            let pawn = self.process.read::<usize>(
                pawn_list_entry + self.offsets.controller_entry * (pawn_handle & 0x1FF) as usize,
            );
            if pawn == 0 || pawn < 0x1000000 || pawn > 0x7FFFFFFFFFFF {
                continue;
            }

            let health = self.process.read::<i32>(pawn + self.offsets.m_iHealth);
            if health <= 0 || health > 100 {
                continue;
            }

            let team = self.process.read::<u8>(pawn + self.offsets.m_iTeamNum);
            let scene_node = self
                .process
                .read::<usize>(pawn + self.offsets.m_pGameSceneNode);
            if scene_node == 0 {
                continue;
            }
            let pos = self
                .process
                .read::<[f32; 3]>(scene_node + self.offsets.m_vecAbsOrigin);

            let mut name = String::from("Unknown");
            let mut name_bytes = [0u8; 32];
            if self.process.read_raw(
                controller + self.offsets.m_sSanitizedPlayerName,
                name_bytes.as_mut_ptr() as _,
                name_bytes.len(),
            ) {
                if let Ok(s) = CStr::from_bytes_until_nul(&name_bytes) {
                    let n = s.to_string_lossy();
                    if !n.is_empty() && n.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                        name = n.into_owned();
                    }
                }
            }

            players.push(Player {
                name,
                health,
                team,
                pos,
            });
        }

        players
    }
}
