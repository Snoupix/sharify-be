use serde::{Deserialize, Serialize};

pub mod endpoints {
    pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
    pub const RECENTLY_PLAYED_TRACKS: &str = "https://api.spotify.com/v1/me/player/recently-played";
    pub const CURRENT_PLAYBACK_STATE: &str = "https://api.spotify.com/v1/me/player";
    pub const PLAYER_QUEUE: &str = "https://api.spotify.com/v1/me/player/queue";
    pub const SEARCH: &str = "https://api.spotify.com/v1/search";
    pub const ADD_TO_QUEUE: &str = "https://api.spotify.com/v1/me/player/queue";
    pub const SET_VOLUME: &str = "https://api.spotify.com/v1/me/player/volume";
    pub const SEEK_TO_POS: &str = "https://api.spotify.com/v1/me/player/seek";
    pub const SKIP_PREVIOUS: &str = "https://api.spotify.com/v1/me/player/previous";
    pub const SKIP_NEXT: &str = "https://api.spotify.com/v1/me/player/next";
    pub const PLAY_RESUME: &str = "https://api.spotify.com/v1/me/player/play";
    pub const PAUSE: &str = "https://api.spotify.com/v1/me/player/pause";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshTokenOutput {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpotifyTrack {
    pub track_id: String,
    pub track_name: String,
    pub artist_name: String,
    pub track_duration: i64,
}

pub type SpotifyTackArray = Vec<SpotifyTrack>;

#[derive(Serialize, Deserialize, Debug)]
pub struct SpotifyPlaylist {
    pub title: String,
    pub tracks: Vec<SpotifyTrack>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct SpotifyCurrentPlaybackOutput {
    pub device_id: String,
    pub device_volume: u8,
    pub shuffle: bool,
    pub progress_ms: Option<u64>,
    pub duration_ms: u64,
    pub is_playing: bool,
    pub track_id: String,
    pub track_name: String,
    pub artist_name: String,
    pub album_image_src: String,
}
