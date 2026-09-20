use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use super::Player;
use crate::models::StreamTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Load(StreamTarget),
    SetPause(bool),
    Stop,
}

/// Records the commands it receives instead of playing anything.
#[derive(Debug, Default)]
pub struct FakePlayer {
    calls: Mutex<Vec<Call>>,
}

impl FakePlayer {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// URLs passed to `load`, in order.
    pub fn loaded_urls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Load(t) => Some(t.url),
                _ => None,
            })
            .collect()
    }

    fn record(&self, call: Call) -> Result<()> {
        self.calls.lock().unwrap().push(call);
        Ok(())
    }
}

#[async_trait]
impl Player for FakePlayer {
    async fn load(&self, target: &StreamTarget) -> Result<()> {
        self.record(Call::Load(target.clone()))
    }

    async fn set_pause(&self, paused: bool) -> Result<()> {
        self.record(Call::SetPause(paused))
    }

    async fn stop(&self) -> Result<()> {
        self.record(Call::Stop)
    }
}
