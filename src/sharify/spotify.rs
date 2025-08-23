use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use urlencoding::encode as encode_url;

use super::spotify_web_utils::endpoints::*;
use super::spotify_web_utils::{
    RefreshTokenOutput, SpotifyCurrentPlaybackOutput, SpotifyTackArray, SpotifyTrack,
};

pub const RATE_LIMIT_REQUEST_WINDOW: Duration = Duration::from_secs(30);
pub const REQUEST_COUNT_PER_WINDOW: u8 = 10;

// pub static CODE: OnceLock<Arc<RwLock<String>>> = OnceLock::new();

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timestamp(String);

impl Timestamp {
    pub fn new(t: String) -> Self {
        Self(t)
    }
}

impl From<Timestamp> for i64 {
    fn from(value: Timestamp) -> Self {
        value.0.parse().unwrap()
    }
}

impl From<i64> for Timestamp {
    fn from(i: i64) -> Self {
        Timestamp(i.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum SpotifyError {
    Generic(String),
    RateLimited(u64),
}

impl From<SpotifyError> for String {
    fn from(err: SpotifyError) -> Self {
        match err {
            SpotifyError::Generic(string) => string,
            SpotifyError::RateLimited(time) => format!("Spotify API rate limited for {time}s"),
        }
    }
}

#[derive(Debug)]
pub struct RateLimiter {
    pub current_window: Instant,
    pub request_count_on_window: AtomicU8,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            current_window: Instant::now(),
            request_count_on_window: AtomicU8::new(1),
        }
    }
}

impl RateLimiter {
    pub fn increment(&mut self) -> Result<(), SpotifyError> {
        let elapsed_since_window = self.current_window.elapsed();

        if elapsed_since_window > RATE_LIMIT_REQUEST_WINDOW {
            self.current_window = Instant::now();
            self.request_count_on_window.store(1, Ordering::SeqCst);

            return Ok(());
        }

        if self.request_count_on_window.fetch_add(1, Ordering::Acquire) + 1
            >= REQUEST_COUNT_PER_WINDOW
        {
            return Err(SpotifyError::RateLimited(
                RATE_LIMIT_REQUEST_WINDOW.as_secs() - elapsed_since_window.as_secs(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SpotifyTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: Timestamp,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Default)]
pub struct Spotify {
    client: reqwest::Client, // cannot use the blocking client because it's used in async threads and blocks them with trying to lock
    pub tokens: SpotifyTokens,
    pub rate_limiter: Arc<RwLock<RateLimiter>>,
}

impl Spotify {
    pub fn new(tokens: SpotifyTokens) -> Self {
        Spotify {
            tokens,
            ..Default::default()
        }
    }

    pub async fn fetch_refresh_token(&mut self) -> Result<SpotifyTokens, SpotifyError> {
        let id = dotenvy::var("SPOTIFY_CLIENT_ID").map_err(|err| {
            SpotifyError::Generic(format!("Failed to get Spotify client ID from env: {err}"))
        })?;

        let res = self
            .client
            .post(format!(
                "{}?grant_type=refresh_token&client_id={}&refresh_token={}",
                TOKEN_URL, id, self.tokens.refresh_token,
            ))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!(
                    "Failed to send Spotify refresh token request: {err}"
                ))
            })?;

        if !res.status().is_success() || !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch Spotify token: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: RefreshTokenOutput = res.json().await.map_err(|err| {
            SpotifyError::Generic(format!("Failed to get Spotify token json result: {err}"))
        })?;

        self.tokens = SpotifyTokens {
            access_token: body.access_token,
            refresh_token: body.refresh_token,
            expires_in: Timestamp::from(body.expires_in),
            created_at: Timestamp::from(chrono::Local::now().timestamp()),
        };

        Ok(self.tokens.clone())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-recently-played
    pub async fn get_recent_tracks(
        &self,
        number: Option<u16>,
    ) -> Result<SpotifyTackArray, SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let number = number.unwrap_or(5);
        if !(1..=50).contains(&number) {
            return Err(SpotifyError::Generic(
                "You must get 1 to 50 recent tracks".into(),
            ));
        }

        let mut output = Vec::new();

        let res = self
            .client
            .get(format!("{RECENTLY_PLAYED_TRACKS}/?limit={number}"))
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!(
                    "Failed to send Spotify {number} recently played tracks request: {err}"
                ))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch {} recent tracks: ({}) {:?}",
                number,
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: serde_json::Value = res.json().await.map_err(|err| {
            SpotifyError::Generic(format!("Failed to parse recent tracks json result: {err}"))
        })?;

        let Some(items) = body["items"].as_array() else {
            error!("Unexpected error: Cannot get items from json output {body:?}");
            return Err(SpotifyError::Generic(
                "Unexpected error: Cannot get items from json output".into(),
            ));
        };

        for item in items {
            output.push(SpotifyTrack {
                track_id: item["track"]["id"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track ID".into()))?
                    .to_owned(),
                track_name: item["track"]["name"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track name".into()))?
                    .to_owned(),
                artist_name: item["track"]["artists"]
                    .as_array()
                    .ok_or(SpotifyError::Generic("Cannot get track artists".into()))?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: item["track"]["duration_ms"]
                    .as_i64()
                    .ok_or(SpotifyError::Generic("Cannot get track duration".into()))?
                    .to_owned(),
            });
        }

        Ok(output)
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-information-about-the-users-current-playback
    pub async fn get_current_playback_state(
        &self,
    ) -> Result<Option<SpotifyCurrentPlaybackOutput>, SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .get(CURRENT_PLAYBACK_STATE)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!(
                    "Failed to send Spotify current playback state request: {err}"
                ))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch current playback state: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: serde_json::Value = match res.json().await {
            Ok(v) => v,
            Err(err) => {
                debug!("Failed to parse current playback state json result (probably empty body because client is not playing): {err}");
                return Ok(None);
            }
        };

        Ok(Some(SpotifyCurrentPlaybackOutput {
            device_id: body["device"]["id"]
                .as_str()
                .ok_or(SpotifyError::Generic("Cannot get device ID".into()))?
                .to_owned(),
            device_volume: body["device"]["volume_percent"]
                .as_u64()
                .ok_or(SpotifyError::Generic("Cannot get device ID".into()))?
                as _,
            shuffle: body["shuffle_state"]
                .as_bool()
                .ok_or(SpotifyError::Generic("Cannot get shuffle state".into()))?,
            progress_ms: if body["progress_ms"].is_null() {
                None
            } else {
                Some(
                    body["progress_ms"]
                        .as_u64()
                        .ok_or(SpotifyError::Generic("Cannot get progress ms".into()))?
                        as _,
                )
            },
            duration_ms: body["item"]["duration_ms"]
                .as_u64()
                .ok_or(SpotifyError::Generic("Cannot get track duration ms".into()))?,
            is_playing: body["is_playing"]
                .as_bool()
                .ok_or(SpotifyError::Generic("Cannot get is playing state".into()))?,
            track_id: body["item"]["id"]
                .as_str()
                .ok_or(SpotifyError::Generic("Cannot get track ID".into()))?
                .to_owned(),
            track_name: body["item"]["name"]
                .as_str()
                .ok_or(SpotifyError::Generic("Cannot get track name".into()))?
                .to_owned(),
            artist_name: body["item"]["artists"]
                .as_array()
                .ok_or(SpotifyError::Generic("Cannot get track artists".into()))?
                .iter()
                .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                .collect::<Vec<_>>()
                .join(" - "),
            album_image_src: body["item"]["album"]["images"]
                .as_array()
                .ok_or(SpotifyError::Generic("Cannot get album image".into()))?
                .first()
                .ok_or(SpotifyError::Generic("Cannot get first album cover".into()))?["url"]
                .as_str()
                .ok_or(SpotifyError::Generic(
                    "Cannot get url field on first album cover image".into(),
                ))?
                .to_owned(),
        }))
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-queue
    pub async fn get_next_tracks(&self) -> Result<SpotifyTackArray, SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let mut output = Vec::new();

        let res = self
            .client
            .get(PLAYER_QUEUE)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send player queue request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch player queue: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: serde_json::Value = res.json().await.map_err(|err| {
            SpotifyError::Generic(format!("Failed to parse next tracks json result: {err}"))
        })?;

        let Some(items) = body["queue"].as_array() else {
            error!("Unexpected error: Cannot get items from json output {body:?}");
            return Err(SpotifyError::Generic(
                "Unexpected error: Cannot get items from json output".into(),
            ));
        };

        for item in items {
            output.push(SpotifyTrack {
                track_id: item["id"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track ID".into()))?
                    .to_owned(),
                track_name: item["name"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track name".into()))?
                    .to_owned(),
                artist_name: item["artists"]
                    .as_array()
                    .ok_or(SpotifyError::Generic("Cannot get track artists".into()))?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: item["duration_ms"]
                    .as_i64()
                    .ok_or(SpotifyError::Generic("Cannot get track duration".into()))?,
            });
        }

        Ok(output)
    }

    // https://developer.spotify.com/documentation/web-api/reference/search
    pub async fn search_track(&self, value: String) -> Result<SpotifyTackArray, SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let mut tracks = Vec::new();

        let res = self
            .client
            .get(format!(
                "{SEARCH}?type=track&q={}&limit=20",
                encode_url(&value)
            ))
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send search request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch search: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: serde_json::Value = res.json().await.map_err(|err| {
            SpotifyError::Generic(format!("Failed to parse search json result: {err}"))
        })?;

        for track in body["tracks"]["items"]
            .as_array()
            .ok_or(SpotifyError::Generic("Cannot parse tracks to array".into()))?
        {
            tracks.push(SpotifyTrack {
                track_id: track["id"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track id".into()))?
                    .to_owned(),
                track_name: track["name"]
                    .as_str()
                    .ok_or(SpotifyError::Generic("Cannot get track name".into()))?
                    .to_owned(),
                artist_name: track["artists"]
                    .as_array()
                    .ok_or(SpotifyError::Generic("Cannot get track artists".into()))?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: track["duration_ms"]
                    .as_i64()
                    .ok_or(SpotifyError::Generic("Cannot get track duration".into()))?,
            })
        }

        Ok(tracks)
    }

    // https://developer.spotify.com/documentation/web-api/reference/add-to-queue
    pub async fn add_track_to_queue(&self, track_id: String) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .post(format!(
                "{ADD_TO_QUEUE}?uri={}",
                encode_url(&format!("spotify:track:{track_id}"))
            ))
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send add to queue request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch add to queue: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback
    pub async fn play_resume(&self) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .put(PLAY_RESUME)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send play resume request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch play resume: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/pause-a-users-playback
    pub async fn pause(&self) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .put(PAUSE)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| SpotifyError::Generic(format!("Failed to send pause request: {err}")))?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch pause: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/skip-users-playback-to-previous-track
    pub async fn skip_previous(&self) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .post(SKIP_PREVIOUS)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send skip to previous request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch skip to previous: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/skip-users-playback-to-next-track
    pub async fn skip_next(&self) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .post(SKIP_NEXT)
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send skip to next request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch skip to next: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/seek-to-position-in-currently-playing-track
    pub async fn seek_to_ms(&self, ms: u64) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .put(format!("{SEEK_TO_POS}?position_ms={}", ms))
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send seek to pos request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch seek to pos: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/set-volume-for-users-playback
    pub async fn set_volume(&self, volume: u8) -> Result<(), SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .put(format!("{SET_VOLUME}?volume_percent={}", volume))
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .header("Content-Length", 0)
            .send()
            .await
            .map_err(|err| SpotifyError::Generic(format!("Failed to set volume request: {err}")))?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch set volume: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        Ok(())
    }

    pub async fn get_my_id(&self) -> Result<String, SpotifyError> {
        self.rate_limiter.write().await.increment()?;

        let res = self
            .client
            .get("https://api.spotify.com/v1/me")
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| {
                SpotifyError::Generic(format!("Failed to send Spotify user info request: {err}"))
            })?;

        if !res.status().is_success() {
            return Err(SpotifyError::Generic(format!(
                "Failed to fetch Spotify user info: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            )));
        }

        let body: serde_json::Value = res.json().await.map_err(|err| {
            SpotifyError::Generic(format!(
                "Failed to parse Spotify user info json result: {err}"
            ))
        })?;

        Ok(body["id"].as_str().unwrap().to_owned())
    }
}
