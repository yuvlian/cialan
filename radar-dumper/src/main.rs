use radar_dumper::{Config, extract_overviews, extract_radars};
use std::fs;
use std::path::Path;
use vpk::Vpk;

fn main() {
    if !Path::new(Config::PATH).exists() {
        let config = Config::default();
        let toml_string = toml::to_string_pretty(&config).unwrap();
        match fs::write(Config::PATH, toml_string) {
            Ok(_) => println!(
                "Created default {}, edit file as needed, then rerun.",
                Config::PATH
            ),
            Err(e) => println!("Failed to write {}: {:?}", Config::PATH, e),
        }
        return;
    }

    let config_content = match fs::read_to_string(Config::PATH) {
        Ok(content) => content,
        Err(e) => {
            println!("Failed to read {}: {:?}", Config::PATH, e);
            return;
        }
    };

    let config = match toml::from_str::<Config>(&config_content) {
        Ok(config) => config,
        Err(e) => {
            println!("Failed to parse {}: {:?}", Config::PATH, e);
            return;
        }
    };

    let vrf_cli_path = Path::new(&config.vrf_cli_path);
    if !vrf_cli_path.exists() {
        println!("Error: VRF CLI not found at {:?}", vrf_cli_path);
        println!(
            "Get it here: https://github.com/ValveResourceFormat/ValveResourceFormat/releases/tag/19.1"
        );
        return;
    }

    let vpk_path = Path::new(&config.vpk_path);
    let assets_dir = Path::new(&config.assets_dir);
    let maps_dir = assets_dir.join("radar");

    println!("Opening VPK: {:?}", vpk_path);
    let vpk = match Vpk::open(vpk_path) {
        Ok(vpk) => vpk,
        Err(e) => {
            println!("Failed to open VPK: {:?}", e);
            return;
        }
    };

    println!("Extracting overviews to {:?}", maps_dir);
    if let Err(e) = extract_overviews(&vpk, &maps_dir) {
        println!("Failed to extract overviews: {:?}", e);
        return;
    }

    println!("Extracting radars to {:?}", maps_dir);
    if let Err(e) = extract_radars(vpk_path, vrf_cli_path, assets_dir, &maps_dir) {
        println!("Failed to extract radars: {:?}", e);
        return;
    }

    println!("Done!");
}
