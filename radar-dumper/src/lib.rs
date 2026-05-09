use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use vpk::Vpk;

static RE_RADAR_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_radar.*$").unwrap());
static RE_KV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)"\s+"([^"]+)""#).unwrap());

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub vpk_path: String,
    pub vrf_cli_path: String,
    pub assets_dir: String,
}

impl Config {
    pub const PATH: &str = "radar-dumper.toml";

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

impl Default for Config {
    fn default() -> Self {
        Self {
            vpk_path: String::from(
                r"C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive\game\csgo\pak01_dir.vpk",
            ),
            vrf_cli_path: String::from(r".\.vrf\Source2Viewer-CLI.exe"),
            assets_dir: String::from(r".\.assets"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerticalSection {
    pub name: String,
    pub altitude_max: f32,
    pub altitude_min: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Overview {
    pub map_name: String,
    pub material: String,
    pub pos_x: f32,
    pub pos_y: f32,
    pub scale: f32,
    pub vertical_sections: Vec<VerticalSection>,
    pub settings: HashMap<String, String>,
}

impl Overview {
    pub fn parse(content: &str) -> Option<Self> {
        let mut overview = Overview::default();
        let mut stack = Vec::new();
        let mut last_name = String::new();

        for line in content.lines() {
            let mut line = line.trim();
            if let Some(pos) = line.find("//") {
                line = &line[..pos].trim();
            }
            if line.is_empty() {
                continue;
            }

            if line == "{" {
                stack.push(last_name.clone());
                if overview.map_name.is_empty() && stack.len() == 1 {
                    overview.map_name = last_name.clone();
                }
                continue;
            }

            if line == "}" {
                stack.pop();
                continue;
            }

            if let Some(cap) = RE_KV.captures(line) {
                let key = &cap[1];
                let val = &cap[2];

                if stack.len() == 1 {
                    match key {
                        "material" => overview.material = val.to_string(),
                        "pos_x" => overview.pos_x = val.parse().unwrap_or(0.0),
                        "pos_y" => overview.pos_y = val.parse().unwrap_or(0.0),
                        "scale" => overview.scale = val.parse().unwrap_or(1.0),
                        _ => {
                            overview.settings.insert(key.to_string(), val.to_string());
                        }
                    }
                } else if stack.len() == 3 && stack[1] == "verticalsections" {
                    let section_name = &stack[2];
                    if overview.vertical_sections.is_empty()
                        || overview.vertical_sections.last().unwrap().name != *section_name
                    {
                        overview.vertical_sections.push(VerticalSection {
                            name: section_name.clone(),
                            ..Default::default()
                        });
                    }
                    let section = overview.vertical_sections.last_mut().unwrap();
                    match key {
                        "AltitudeMax" => section.altitude_max = val.parse().unwrap_or(0.0),
                        "AltitudeMin" => section.altitude_min = val.parse().unwrap_or(0.0),
                        _ => {}
                    }
                }
            } else {
                last_name = line.trim_matches('"').to_string();
            }
        }

        if overview.map_name.is_empty() {
            None
        } else {
            Some(overview)
        }
    }
}

pub fn extract_overviews(vpk: &Vpk, out_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;

    for path in vpk.tree.keys() {
        if path.starts_with("resource/overviews/") && path.ends_with(".txt") {
            let stem = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap();
            let out_name = format!("{}_radar.txt", stem);
            let out_path = out_dir.join(out_name);

            let data = vpk.get_file_content(path)?;

            let write = if out_path.exists() {
                fs::read(&out_path)? != data
            } else {
                true
            };

            if write {
                fs::write(out_path, data)?;
            }
        }
    }
    Ok(())
}

pub fn extract_radars(
    vpk_path: &Path,
    cli_path: &Path,
    assets_root: &Path,
    out_dir: &Path,
) -> io::Result<()> {
    let tmp_out = assets_root.join("tmp_radar");
    fs::create_dir_all(&tmp_out)?;
    fs::create_dir_all(out_dir)?;

    let mut cmd = Command::new(cli_path);
    cmd.arg("-i")
        .arg(vpk_path)
        .arg("-o")
        .arg(&tmp_out)
        .arg("-d")
        .arg("--vpk_extensions")
        .arg("vtex_c")
        .arg("--vpk_filepath")
        .arg("panorama/images/overheadmaps/");

    let output = cmd.output()?;
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "VRF CLI failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }

    let mut candidates = Vec::new();
    find_files(&tmp_out, &mut candidates)?;
    candidates.sort();

    for img_file in candidates {
        let stem = img_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !stem.contains("_radar") {
            continue;
        }

        let out_name = format!("{}.png", RE_RADAR_SUFFIX.replace(stem, "_radar"));
        let dest_path = out_dir.join(out_name);

        let data = fs::read(&img_file)?;
        let write = if dest_path.exists() {
            fs::read(&dest_path)? != data
        } else {
            true
        };

        if write {
            fs::write(dest_path, data)?;
        }
    }

    let _ = fs::remove_dir_all(tmp_out);

    Ok(())
}

fn find_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                find_files(&path, files)?;
            } else {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "png" || ext == "tga" {
                    files.push(path);
                }
            }
        }
    }
    Ok(())
}
