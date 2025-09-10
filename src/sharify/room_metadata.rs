use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::spotify::{Spotify, SpotifyTokens};

#[derive(Clone, Debug)]
pub struct RoomMetadata {
    pub inactive_for: Option<Instant>,
    pub spotify_handler: Spotify,

    spotify_data_sleeper: Option<mpsc::Sender<Duration>>,
}

impl RoomMetadata {
    pub fn new(spotify_tokens: SpotifyTokens) -> Self {
        Self {
            spotify_handler: Spotify::new(spotify_tokens),
            inactive_for: None,
            spotify_data_sleeper: None,
        }
    }

    pub fn init_spotify_tick_tx(&mut self, tx: mpsc::Sender<Duration>) {
        self.spotify_data_sleeper = Some(tx);
    }

    pub async fn set_spotify_tick(&mut self, tick: Duration) {
        if let Some(sleeper) = self.spotify_data_sleeper.as_ref() {
            if let Err(err) = sleeper.send(tick).await {
                error!("An error occured while trying to send the new tick {err}");
            }

            return;
        }

        error!("Unreachable error: Trying to set the sleep duration when it has not been (yet) defined");
    }
}
