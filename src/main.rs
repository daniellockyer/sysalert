#![feature(ip)]

use std::fs;
use std::net::IpAddr;

use local_ip_address::list_afinet_netifas;
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
    memory: Memory,
    #[serde(default)]
    disks: Disks,
    #[serde(default)]
    load_average: LoadAverage,
    #[serde(default)]
    process_checks: ProcessChecks,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessChecks {
    #[serde(default)]
    disable_mysql_check: bool,
    #[serde(default)]
    disable_mysql_memory_check: bool,
    #[serde(default)]
    disable_web_server_check: bool,
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
    system.cpus().len() as f64
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

    match reqwest::blocking::Client::new().post(url).json(&map).send() {
        Ok(r) => match r.status() {
            reqwest::StatusCode::OK => (),
            _ => eprintln!("{:#?}", r.text()),
        },
        Err(e) => eprintln!("{e:#?}"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_file = std::env::var("CONFIG").unwrap_or_else(|_| "sysalert.toml".to_string());
    let config: Config = toml::from_str(&std::fs::read_to_string(config_file)?)?;

    println!("sysalert v{}", cargo_crate_version!());
    println!("{config:#?}");

    let s = System::new_all();
    let hostname = dbg!(s.host_name().unwrap_or_else(|| "unknown".to_string()));

    let ip_addr = if let Ok(ifas) = list_afinet_netifas() {
        if let Some((_, ipaddr)) = ifas
            .iter()
            .find(|(_, ipaddr)| ipaddr.is_global() && matches!(ipaddr, IpAddr::V4(_)))
        {
            format!("{ipaddr:?}")
        } else {
            "unknown".to_owned()
        }
    } else {
        "unknown".to_owned()
    };

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
            let mut message = format!("✅ `{hostname} ({ip_addr}) updated to v{version}`");

            // read contents from /var/spool/cron/crontabs/root file
            match std::fs::read_to_string("/var/spool/cron/crontabs/root") {
                Ok(result) => {
                    let lines = result
                        .lines()
                        .filter(|line| line.contains("b2"))
                        .map(String::from)
                        .collect::<Vec<String>>()
                        .join("\n");

                    message += &format!("\n```{lines}```");
                }
                Err(e) => {
                    message += &format!("\n```{}```", e);
                }
            }

            send_telegram(&config, message);
        }
    }

    let mut errors = Vec::new();

    macro_rules! check_value {
        ($name:expr, $value:expr, $sign:tt, $threshold:expr) => {
            if $value $sign $threshold {
                errors.push(format!(
                    "`{}: {:.4} {} {}`",
                    $name,
                    $value,
                    stringify!($sign),
                    $threshold
                ));
            }
        };
    }

    macro_rules! check_running {
        ($config:expr, $names:expr) => {
            if !$config {
                let mut is_running = false;

                for name in $names {
                    let process_count = s.processes_by_name(name);
                    let mut i = 0;

                    for _ in process_count {
                        i += 1;
                    }

                    if i != 0 {
                        is_running = true;
                    }
                }

                if is_running {
                    println!("{} is running", $names.join(", "));
                } else {
                    errors.push(format!("`{} is not running`", $names.join(", ")));
                }
            }
        };
    }

    let system_load_avg = dbg!(s.load_average());
    check_value!("🚨 load 1", system_load_avg.one, >, config.load_average.one * 2.0);
    check_value!("load 5", system_load_avg.five, >, config.load_average.five);
    check_value!("load 15", system_load_avg.fifteen, >, config.load_average.fifteen);

    let disks = dbg!(s.disks());
    for d in disks {
        let mount = format!("{}", d.mount_point().to_string_lossy());

        if config.disks.disks.contains(&mount) {
            let perc_free = dbg!(d.available_space() as f64 / d.total_space() as f64);
            check_value!(mount, perc_free, <, config.disks.minimum);
        }
    }

    let memory_perc_free = if s.available_memory() as f64 == 0.0 {
        dbg!((s.total_memory() - s.used_memory()) as f64 / s.total_memory() as f64)
    } else {
        dbg!(s.available_memory() as f64 / s.total_memory() as f64)
    };
    check_value!("memory", memory_perc_free, <, config.memory.minimum);

    check_running!(
        config.process_checks.disable_web_server_check,
        &["apache2", "nginx"]
    );
    check_running!(
        config.process_checks.disable_mysql_check,
        &["mariadbd", "mysqld"]
    );

    if !config.process_checks.disable_mysql_memory_check {
        for process in s.processes_by_name("mysqld") {
            let process_memory = dbg!(process.memory());
            check_value!("mysqld", process_memory, >, (s.total_memory() as f64 * 0.75) as u64);
        }
        for process in s.processes_by_name("mariadbd") {
            let process_memory = dbg!(process.memory());
            check_value!("mariadbd", process_memory, >, (s.total_memory() as f64 * 0.75) as u64);
        }
    }

    let b2_processes_count = s
        .processes()
        .values()
        .filter(|p| p.name().contains("b2"))
        .count();

    if b2_processes_count > 1 {
        errors.push(format!("`b2 is running {} times`", b2_processes_count));
    }

    let backup_file = "/tmp/backup.heartbeat";

    match fs::metadata(backup_file) {
        Ok(metadata) => {
            if let Ok(time) = metadata.modified() {
                if let Ok(time_sec) = time.elapsed() {
                    // If the time is older than 24 hours and 15 minutes, add an alert
                    if time_sec.as_secs() > ((60 * 60 * 24) + (60 * 15)) {
                        errors.push(format!("`{} has expired`", backup_file));
                    }
                } else {
                    errors.push(format!("`{}`", "Heartbeat timestamp not supported"));
                }
            } else {
                errors.push(format!("`{}`", "Heartbeat timestamp not supported"));
            }
        }
        Err(e) => errors.push(format!("`Heartbeat error: {}`", e)),
    }

    let time_mins = 10;
    if s.uptime() < time_mins * 60 {
        errors.push(format!("rebooted within the last {time_mins}"));
    }

    if !errors.is_empty() {
        send_telegram(
            &config,
            format!("❗ `{}` \\- `{}`\n{}", hostname, ip_addr, errors.join("\n")),
        );
    }

    Ok(())
}
