use serde::Deserialize;
use std::collections::HashMap;
use sysinfo::{DiskExt, System, SystemExt};

#[derive(Deserialize)]
struct Config {
    telegram_token: String,
    telegram_chat_id: String,
    load_average: Option<LoadAverage>,
    disks: Option<Disks>,
}

#[derive(Deserialize)]
struct LoadAverage {
    one_max: Option<f64>,
    five_max: Option<f64>,
    fifteen_max: Option<f64>,
}

#[derive(Deserialize)]
struct Disks {
    disks: Vec<String>,
    minimum: Option<f64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_file = std::env::var("CONFIG").unwrap_or("sysalert.toml".to_string());
    let config: Config = toml::from_str(&std::fs::read_to_string(config_file)?)?;

    let s = System::new_all();
    let hostname = dbg!(hostname::get()?.into_string().unwrap());
    let mut errors = Vec::new();

    macro_rules! check_value {
        ($name:expr, $value:expr, $sign:tt, $threshold:expr) => {
            if let Some(t) = $threshold {
                if $value $sign t {
                    errors.push(format!(
                        "*{}*: `value {} is {} threshold {}`",
                        $name,
                        $value,
                        stringify!($sign),
                        t
                    ));
                }
            }
        };
    }

    if let Some(config_load_average) = config.load_average {
        let system_load_avg = dbg!(s.get_load_average());

        check_value!("load average, one", system_load_avg.one, >, config_load_average.one_max);
        check_value!("load average, five", system_load_avg.five, >, config_load_average.five_max);
        check_value!(
            "load_average, fifteen",
            system_load_avg.fifteen,
            >,
            config_load_average.fifteen_max
        );
    }

    if let Some(config_disks) = config.disks {
        let system_disks = dbg!(s.get_disks());

        for d in system_disks {
            let mount = format!("{}", d.get_mount_point().to_string_lossy());

            if config_disks.disks.contains(&mount) {
                let perc_free = dbg!((d.get_available_space()) as f64 / d.get_total_space() as f64);
                let name = format!("mount {}", mount);

                check_value!(name, perc_free, <, config_disks.minimum);
            }
        }
    }

    if errors.len() == 0 {
        return Ok(());
    }

    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        config.telegram_token
    );

    let mut map = HashMap::new();
    map.insert("chat_id", config.telegram_chat_id);
    map.insert("parse_mode", "MarkdownV2".to_string());
    map.insert("disable_web_page_preview", "true".to_string());
    map.insert("text", format!("❗ `{}`:\n{}", hostname, errors.join("\n")));

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

    Ok(())
}
