use std::io::{Result, Write};
use std::path::{Path, PathBuf};
use toml_edit::{Decor, Document, Item, Table, Value};

fn resolve_config_path(platform: Option<&str>) -> Result<PathBuf> {
    let root_dir = PathBuf::from(std::env!("CARGO_MANIFEST_DIR"));
    let config_dir = root_dir.join("platforms");

    eprintln!("config_dir: {:?}", config_dir.display());

    let builtin_platforms = std::fs::read_dir(&config_dir)?
        .filter_map(|e| {
            e.unwrap()
                .file_name()
                .to_str()?
                .strip_suffix(".toml")
                .map(String::from)
        })
        .collect::<Vec<_>>();

    let path = match platform {
        None | Some("") => "defconfig.toml".into(),
        Some(plat) if builtin_platforms.contains(&plat.to_string()) => {
            config_dir.join(format!("{plat}.toml"))
        }
        Some(plat) => {
            let path = PathBuf::from(&plat);
            if path.is_absolute() {
                path
            } else {
                root_dir.join(plat)
            }
        }
    };

    Ok(path)
}

fn get_comments<'a>(config: &'a Table, key: &str) -> Option<&'a str> {
    config
        .key_decor(key)
        .and_then(|d| d.prefix())
        .and_then(|s| s.as_str())
        .map(|s| s.trim())
}

fn add_config(config: &mut Table, key: &str, item: Item, comments: Option<&str>) {
    config.insert(key, item);
    if let Some(comm) = comments {
        if let Some(dst) = config.key_decor_mut(key) {
            *dst = Decor::new(comm, "");
        }
    }
}

fn load_config_toml(config_path: &Path) -> Result<Table> {
    let config_content = std::fs::read_to_string(config_path)?;
    let toml = config_content
        .parse::<Document>()
        .expect("failed to parse config file")
        .as_table()
        .clone();
    Ok(toml)
}

fn gen_config_rs(config_path: &Path) -> Result<Vec<u8>> {
    fn is_num(s: &str) -> bool {
        let s = s.replace('_', "");
        if s.parse::<usize>().is_ok() {
            true
        } else if let Some(s) = s.strip_prefix("0x") {
            usize::from_str_radix(s, 16).is_ok()
        } else {
            false
        }
    }

    // Load TOML config file
    let mut config = if config_path == Path::new("defconfig.toml") {
        load_config_toml(config_path)?
    } else {
        // Set default values for missing items
        let defconfig = load_config_toml(Path::new("defconfig.toml"))?;
        let mut config = load_config_toml(config_path)?;

        for (key, item) in defconfig.iter() {
            if !config.contains_key(key) {
                add_config(
                    &mut config,
                    key,
                    item.clone(),
                    get_comments(&defconfig, key),
                );
            }
        }
        config
    };

    add_config(
        &mut config,
        "smp",
        toml_edit::value(std::env::var("AX_SMP").unwrap_or("1".into())),
        Some("# Number of CPUs"),
    );

    // Generate config.rs
    let mut output = Vec::new();
    writeln!(
        output,
        "// Platform constants and parameters for {}.",
        config["platform"].as_str().unwrap(),
    )?;
    writeln!(output, "// Generated by build.rs, DO NOT edit!\n")?;

    for (key, item) in config.iter() {
        let var_name = key.to_uppercase().replace('-', "_");
        if let Item::Value(value) = item {
            let comments = get_comments(&config, key)
                .unwrap_or_default()
                .replace('#', "///");
            match value {
                Value::String(s) => {
                    writeln!(output, "{comments}")?;
                    let s = s.value();
                    if is_num(s) {
                        writeln!(output, "pub const {var_name}: usize = {s};")?;
                    } else {
                        writeln!(output, "pub const {var_name}: &str = \"{s}\";")?;
                    }
                }
                Value::Array(regions) => {
                    if key != "mmio-regions" && key != "virtio-mmio-regions" && key != "pci-ranges"
                    {
                        continue;
                    }
                    writeln!(output, "{comments}")?;
                    writeln!(output, "pub const {var_name}: &[(usize, usize)] = &[")?;
                    for r in regions.iter() {
                        let r = r.as_array().unwrap();
                        writeln!(
                            output,
                            "    ({}, {}),",
                            r.get(0).unwrap().as_str().unwrap(),
                            r.get(1).unwrap().as_str().unwrap()
                        )?;
                    }
                    writeln!(output, "];")?;
                }
                _ => {}
            }
        }
    }

    Ok(output)
}

fn main() -> Result<()> {
    let platform = option_env!("AX_PLATFORM");
    let config_path = resolve_config_path(platform)?;

    println!("Reading config file: {:?}", config_path);
    let config_rs = gen_config_rs(&config_path)?;

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("config.rs");
    println!("Generating config file: {}", out_path.display());
    std::fs::write(out_path, config_rs)?;

    println!("cargo:rerun-if-changed={}", config_path.display());
    println!("cargo:rerun-if-env-changed=AX_PLATFORM");
    println!("cargo:rerun-if-env-changed=AX_SMP");
    Ok(())
}
