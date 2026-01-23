use crate::job::Job;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JobStore {
    pub jobs: HashMap<String, Job>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dnd_until: Option<DateTime<Utc>>,
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("usched")
}

fn jobs_path() -> PathBuf {
    data_dir().join("jobs.json")
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

impl JobStore {
    pub fn load() -> Result<Self> {
        let path = jobs_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let store: JobStore = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        let dir = data_dir();
        fs::create_dir_all(&dir)?;
        let path = jobs_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn add(&mut self, job: Job) {
        self.jobs.insert(job.id.clone(), job);
    }

    pub fn remove(&mut self, id: &str) -> Option<Job> {
        self.jobs.remove(id)
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.get_mut(id)
    }

    pub fn list(&self) -> Vec<&Job> {
        let mut jobs: Vec<_> = self.jobs.values().collect();
        jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        jobs
    }
}

impl State {
    pub fn load() -> Result<Self> {
        let path = state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let state: State = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(state)
    }

    pub fn save(&self) -> Result<()> {
        let dir = data_dir();
        fs::create_dir_all(&dir)?;
        let path = state_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn is_dnd_active(&self) -> bool {
        if let Some(until) = self.dnd_until {
            Utc::now() < until
        } else {
            false
        }
    }

    pub fn set_dnd(&mut self, until: DateTime<Utc>) {
        self.dnd_until = Some(until);
    }

    pub fn clear_dnd(&mut self) {
        self.dnd_until = None;
    }
}

pub fn get_data_dir() -> PathBuf {
    data_dir()
}
