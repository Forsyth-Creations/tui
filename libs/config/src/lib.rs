use serde::{Deserialize, Serialize};
use std::fs;
use console::style;

// Define whatever shape your YAML has
#[derive(Deserialize, Serialize, Debug)]
pub struct ConfigData {
    pub hosts: Vec<String>,
    pub tasks: Vec<String>,
}

pub struct Config {
    pub path: String,
    pub data: ConfigData,
}

impl Config {
    pub fn new(path: Option<String>) -> Result<Self, anyhow::Error> {
        let path = path.unwrap_or("./despereaux.yaml".to_string());
        println!("Loading config from {}", path);
        let contents = fs::read_to_string(&path).map_err(|err| {
            println!("Error reading config file: {}", style(&err).red().bold());
            anyhow::anyhow!("{}", err)
        }
        )?;
        let data: ConfigData = serde_yaml::from_str(&contents).map_err(|e| {
            println!("{} {}", style("error:").red().bold(), e);
            e  // re-return the error so it keeps propagating
        })?;

        Ok(Self {
            path,
            data,
        })
    }
}