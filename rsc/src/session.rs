//! Glue between a backend, a player and a queue: decides what to play next.

use std::sync::Arc;

use anyhow::Result;

use crate::backend::Backend;
use crate::models::Track;
use crate::player::{EndReason, Player, PlayerEvent};
use crate::queue::Queue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// The last track ended (or nothing playable is left).
    Finished,
}

pub struct Session {
    backend: Arc<dyn Backend>,
    player: Arc<dyn Player>,
    queue: Queue,
    paused: bool,
}

impl Session {
    pub fn new(backend: Arc<dyn Backend>, player: Arc<dyn Player>, tracks: Vec<Track>) -> Self {
        Self {
            backend,
            player,
            queue: Queue::new(tracks),
            paused: false,
        }
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Starts playing the first track.
    pub async fn start(&mut self) -> Result<Flow> {
        self.play_current().await
    }

    /// Manual skip forward. Does nothing on the last track.
    pub async fn next(&mut self) -> Result<Flow> {
        if self.queue.advance() {
            self.play_current().await
        } else {
            Ok(Flow::Continue)
        }
    }

    /// Manual skip backward. On the first track it restarts it.
    pub async fn previous(&mut self) -> Result<Flow> {
        self.queue.previous();
        self.play_current().await
    }

    pub async fn toggle_pause(&mut self) -> Result<()> {
        self.paused = !self.paused;
        self.player.set_pause(self.paused).await
    }

    pub async fn handle_event(&mut self, event: PlayerEvent) -> Result<Flow> {
        match event {
            PlayerEvent::EndFile(EndReason::Eof) => self.advance_after_end().await,
            PlayerEvent::EndFile(EndReason::Error) => {
                tracing::warn!("mpv could not play the current track, skipping it");
                self.advance_after_end().await
            }
            // `Stop` is what mpv sends for the track *we* replaced with `loadfile`:
            // advancing here would skip a track on every manual `next`.
            PlayerEvent::EndFile(_) => Ok(Flow::Continue),
            PlayerEvent::Paused(paused) => {
                self.paused = paused;
                Ok(Flow::Continue)
            }
            PlayerEvent::Position(_) | PlayerEvent::Duration(_) => Ok(Flow::Continue),
        }
    }

    async fn advance_after_end(&mut self) -> Result<Flow> {
        if self.queue.advance() {
            self.play_current().await
        } else {
            Ok(Flow::Finished)
        }
    }

    /// Plays the track under the cursor. Tracks whose stream cannot be resolved are
    /// skipped; a player failure is a real error and is returned.
    async fn play_current(&mut self) -> Result<Flow> {
        loop {
            let Some(track) = self.queue.current().cloned() else {
                return Ok(Flow::Finished);
            };
            // Resolved now, not when the playlist was loaded: stream URLs expire.
            match self.backend.stream_target(&track).await {
                Ok(target) => {
                    self.player.load(&target).await?;
                    if self.paused {
                        // mpv keeps its `pause` property across files.
                        self.paused = false;
                        self.player.set_pause(false).await?;
                    }
                    return Ok(Flow::Continue);
                }
                Err(err) => {
                    tracing::warn!("skipping {:?}: {err:#}", track.title);
                    if !self.queue.advance() {
                        return Ok(Flow::Finished);
                    }
                }
            }
        }
    }
}
