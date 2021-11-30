use self_update::{backends::github::Update, cargo_crate_version, Status::Updated};
use serde::Deserialize;
use std::collections::HashMap;
use sysinfo::{DiskExt, ProcessExt, System, SystemExt};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    telegram_token: String,
    telegram_chat_id: String,
    #[serde(default)]
    disable_self_update: bool,
    #[serde(default)]
    disable_mysql_memory_check: bool,
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
    system.processors().len() as f64
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
    let config_file = std::env::var("CONFIG").unwrap_or_else(|_| "sysalert.toml".to_string());
    let config: Config = toml::from_str(&std::fs::read_to_string(config_file)?)?;

    println!("sysalert v{}", cargo_crate_version!());
    println!("{:#?}", config);

    let s = System::new_all();
    let hostname = dbg!(s.host_name().unwrap_or_else(|| "unknown".to_string()));
    let mut errors = Vec::new();

    macro_rules! check_value {
        ($name:expr, $value:expr, $sign:tt, $threshold:expr) => {
            if $value $sign $threshold {
                errors.push(format!(
                    "`{}: {:.4} {} threshold {}`",
                    $name,
                    $value,
                    stringify!($sign),
                    $threshold
                ));
            }
        };
    }

    let system_load_avg = dbg!(s.load_average());
    check_value!("load 1", system_load_avg.one, >, config.load_average.one);
    check_value!("load 5", system_load_avg.five, >, config.load_average.five);
    check_value!("load 15", system_load_avg.fifteen, >, config.load_average.fifteen);

    let disks = dbg!(s.disks());
    for d in disks {
        let mount = format!("{}", d.mount_point().to_string_lossy());

        if config.disks.disks.contains(&mount) {
            let perc_free = dbg!(d.available_space() as f64 / d.total_space() as f64);
            let name = format!("mount {}", mount);

            check_value!(name, perc_free, <, config.disks.minimum);
        }
    }

    let memory_perc_free = if s.available_memory() as f64 == 0.0 {
        dbg!((s.total_memory() - s.used_memory()) as f64 / s.total_memory() as f64)
    } else {
        dbg!(s.available_memory() as f64 / s.total_memory() as f64)
    };
    check_value!("memory", memory_perc_free, <, config.memory.minimum);

    if !config.disable_self_update {
        for process in s.process_by_name("mysqld") {
            check_value!("mysqld", process.memory(), >, s.total_memory() / 2);
        }
        for process in s.process_by_name("mariadbd") {
            check_value!("mariadbd", process.memory(), >, s.total_memory() / 2);
        }
    }

    if !errors.is_empty() {
        send_telegram(
            &config,
            format!("❗ `{}`:\n{}", hostname, errors.join("\n")),
        );
    }

    if !config.disable_self_update {
        if let Updated(version) = Update::configure()
            .repo_owner("daniellockyer")
            .repo_name("sysalert")
            .bin_name("sysalert")
            .target("x86_64-unknown-linux-musl")
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
