use self_update::{cargo_crate_version, Status::Updated};
use serde::Deserialize;
use std::collections::HashMap;
use sysinfo::{DiskExt, System, SystemExt};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    telegram_token: String,
    telegram_chat_id: String,
    #[serde(default)]
    disable_self_update: bool,
    #[serde(default)]
    memory: Memory,
    #[serde(default)]
    disks: Disks,
    #[serde(default)]
    load_average: LoadAverage,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadAverage {
    #[serde(default = "default_load_average")]
    one: f64,
    #[serde(default = "default_load_average")]
    five: f64,
    #[serde(default = "default_load_average")]
    fifteen: f64,
}

fn default_load_average() -> f64 {
    let mut system = System::new();
    system.refresh_cpu();
    system.get_processors().len() as f64
}

impl Default for LoadAverage {
    fn default() -> Self {
        let value = default_load_average();

        Self {
            one: value,
            five: value,
            fifteen: value,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disks {
    #[serde(default = "default_disks")]
    disks: Vec<String>,
    #[serde(default = "default_disks_minimum")]
    minimum: f64,
}

fn default_disks() -> Vec<String> {
    vec!["/".to_string()]
}

fn default_disks_minimum() -> f64 {
    0.05
}

impl Default for Disks {
    fn default() -> Self {
        Self {
            disks: default_disks(),
            minimum: default_disks_minimum(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Memory {
    #[serde(default = "default_memory_minimum")]
    minimum: f64,
}

fn default_memory_minimum() -> f64 {
    0.05
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            minimum: default_memory_minimum(),
        }
    }
}

fn send_telegram(config: &Config, message: String) {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        config.telegram_token
    );

    let mut map = HashMap::new();
    map.insert("chat_id", config.telegram_chat_id.clone());
    map.insert("parse_mode", "MarkdownV2".to_string());
    map.insert("text", message);

    match reqwest::blocking::Client::new()
        .post(&url)
        .json(&map)
        .send()
    {
        Ok(r) => match r.status() {
            reqwest::StatusCode::OK => (),
            _ => eprintln!("{:#?}", r.text()),
        },
        Err(e) => eprintln!("{:#?}", e),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_file = std::env::var("CONFIG").unwrap_or("sysalert.toml".to_string());
    let config: Config = toml::from_str(&std::fs::read_to_string(config_file)?)?;

    println!("sysalert v{}", cargo_crate_version!());
    println!("{:#?}", config);

    let s = System::new_all();
    let hostname = dbg!(hostname::get()?.into_string().unwrap());
    let mut errors = Vec::new();

    macro_rules! check_value {
        ($name:expr, $value:expr, $sign:tt, $threshold:expr) => {
            if $value $sign $threshold {
                errors.push(format!(
                    "`{}: value {:.4} {} threshold {}`",
                    $name,
                    $value,
                    stringify!($sign),
                    $threshold
                ));
            }
        };
    }

    let system_load_avg = dbg!(s.get_load_average());
    check_value!("load average 1", system_load_avg.one, >, config.load_average.one);
    check_value!("load average 5", system_load_avg.five, >, config.load_average.five);
    check_value!(
        "load_average 15",
        system_load_avg.fifteen,
        >,
        config.load_average.fifteen
    );

    for d in dbg!(s.get_disks()) {
        let mount = format!("{}", d.get_mount_point().to_string_lossy());

        if config.disks.disks.contains(&mount) {
            let perc_free = dbg!(d.get_available_space() as f64 / d.get_total_space() as f64);
            let name = format!("mount {}", mount);

            check_value!(name, perc_free, <, config.disks.minimum);
        }
    }

    let memory_perc_free = dbg!(s.get_available_memory() as f64 / s.get_total_memory() as f64);
    check_value!("memory", memory_perc_free, <, config.memory.minimum);

    if errors.len() != 0 {
        send_telegram(
            &config,
            format!("❗ `{}`:\n{}", hostname, errors.join("\n")),
        );
    }

    if !config.disable_self_update {
        if let Updated(version) = self_update::backends::github::Update::configure()
            .repo_owner("daniellockyer")
            .repo_name("sysalert")
            .bin_name("sysalert")
            .show_download_progress(true)
            .no_confirm(true)
            .current_version(cargo_crate_version!())
            .build()?
            .update()?
        {
            send_telegram(&config, format!("✅ `{} updated to {}`", hostname, version));
        }
    }

    Ok(())
}
