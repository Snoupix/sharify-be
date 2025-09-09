use std::time::{Duration, Instant};

use super::spotify::{self, Spotify, SpotifyTokens};

#[derive(Clone, Debug)]
pub struct RoomMetadata {
    pub inactive_for: Option<Instant>,
    pub spotify_handler: Spotify,
    pub spotify_data_tick: Duration,
}

impl RoomMetadata {
    pub fn new(spotify_tokens: SpotifyTokens) -> Self {
        Self {
            spotify_handler: Spotify::new(spotify_tokens),
            inactive_for: None,
            spotify_data_tick: spotify::DEFAULT_DATA_INTERVAL,
        }
    }
}
