use serde::{Deserialize, Serialize};
use serde_json::json;
use urlencoding::encode as encode_url;

use super::spotify_web_utils::endpoints::*;
use super::spotify_web_utils::{
    RefreshTokenOutput, SpotifyCurrentPlaybackOutput, SpotifyPlaylist, SpotifyTackArray,
    SpotifyTrack,
};

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
}

impl Spotify {
    pub fn new(tokens: SpotifyTokens) -> Self {
        Spotify {
            tokens,
            ..Default::default()
        }
    }

    pub async fn fetch_refresh_token(&mut self) -> Result<SpotifyTokens, String> {
        let id = dotenvy::var("SPOTIFY_CLIENT_ID")
            .map_err(|err| format!("Failed to get Spotify client ID from env: {err}"))?;

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
            .map_err(|err| format!("Failed to send Spotify refresh token request: {err}"))?;

        if !res.status().is_success() || !res.status().is_success() {
            return Err(format!(
                "Failed to fetch Spotify token: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        let body: RefreshTokenOutput = res
            .json()
            .await
            .map_err(|err| format!("Failed to get Spotify token json result: {err}"))?;

        self.tokens = SpotifyTokens {
            access_token: body.access_token,
            refresh_token: body.refresh_token,
            expires_in: Timestamp::from(body.expires_in),
            created_at: Timestamp::from(chrono::Local::now().timestamp()),
        };

        Ok(self.tokens.clone())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-recently-played
    pub async fn get_recent_tracks(&self, number: Option<u16>) -> Result<SpotifyTackArray, String> {
        let number = number.unwrap_or(5);
        if !(1..=50).contains(&number) {
            return Err("You must get 1 to 50 recent tracks".into());
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
                format!("Failed to send Spotify {number} recently played tracks request: {err}")
            })?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch {} recent tracks: ({}) {:?}",
                number,
                res.status(),
                res.text().await.unwrap()
            ));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|err| format!("Failed to parse recent tracks json result: {err}"))?;

        let Some(items) = body["items"].as_array() else {
            error!("Unexpected error: Cannot get items from json output {body:?}");
            return Err("Unexpected error: Cannot get items from json output".into());
        };

        for item in items {
            output.push(SpotifyTrack {
                track_id: item["track"]["id"]
                    .as_str()
                    .ok_or("Cannot get track ID")?
                    .to_owned(),
                track_name: item["track"]["name"]
                    .as_str()
                    .ok_or("Cannot get track name")?
                    .to_owned(),
                artist_name: item["track"]["artists"]
                    .as_array()
                    .ok_or("Cannot get track artists")?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: item["track"]["duration_ms"]
                    .as_i64()
                    .ok_or("Cannot get track duration")?
                    .to_owned(),
            });
        }

        Ok(output)
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-information-about-the-users-current-playback
    pub async fn get_current_playback_state(
        &self,
    ) -> Result<Option<SpotifyCurrentPlaybackOutput>, String> {
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
                format!("Failed to send Spotify current playback state request: {err}")
            })?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch current playback state: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
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
                .ok_or("Cannot get device ID")?
                .to_owned(),
            device_volume: body["device"]["volume_percent"]
                .as_u64()
                .ok_or("Cannot get device ID")? as _,
            shuffle: body["shuffle_state"]
                .as_bool()
                .ok_or("Cannot get shuffle state")?,
            progress_ms: if body["progress_ms"].is_null() {
                None
            } else {
                Some(
                    body["progress_ms"]
                        .as_u64()
                        .ok_or("Cannot get progress ms")? as _,
                )
            },
            duration_ms: body["item"]["duration_ms"]
                .as_u64()
                .ok_or("Cannot get track duration ms")?,
            is_playing: body["is_playing"]
                .as_bool()
                .ok_or("Cannot get is playing state")?,
            track_id: body["item"]["id"]
                .as_str()
                .ok_or("Cannot get track ID")?
                .to_owned(),
            track_name: body["item"]["name"]
                .as_str()
                .ok_or("Cannot get track name")?
                .to_owned(),
            artist_name: body["item"]["artists"]
                .as_array()
                .ok_or("Cannot get track artists")?
                .iter()
                .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                .collect::<Vec<_>>()
                .join(" - "),
            album_image_src: body["item"]["album"]["images"]
                .as_array()
                .ok_or("Cannot get album image")?
                .first()
                .ok_or("Cannot get first album cover")?["url"]
                .as_str()
                .ok_or("Cannot get url field on first album cover image")?
                .to_owned(),
        }))
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-queue
    pub async fn get_next_tracks(&self) -> Result<SpotifyTackArray, String> {
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
            .map_err(|err| format!("Failed to send player queue request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch player queue: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|err| format!("Failed to parse next tracks json result: {err}"))?;

        let Some(items) = body["queue"].as_array() else {
            error!("Unexpected error: Cannot get items from json output {body:?}");
            return Err("Unexpected error: Cannot get items from json output".into());
        };

        for item in items {
            output.push(SpotifyTrack {
                track_id: item["id"].as_str().ok_or("Cannot get track ID")?.to_owned(),
                track_name: item["name"]
                    .as_str()
                    .ok_or("Cannot get track name")?
                    .to_owned(),
                artist_name: item["artists"]
                    .as_array()
                    .ok_or("Cannot get track artists")?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: item["duration_ms"]
                    .as_i64()
                    .ok_or("Cannot get track duration")?,
            });
        }

        Ok(output)
    }

    // https://developer.spotify.com/documentation/web-api/reference/search
    pub async fn search_track(&self, value: String) -> Result<SpotifyTackArray, String> {
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
            .map_err(|err| format!("Failed to send search request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch search: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|err| format!("Failed to parse search json result: {err}"))?;

        for track in body["tracks"]["items"]
            .as_array()
            .ok_or("Cannot parse tracks to array")?
        {
            tracks.push(SpotifyTrack {
                track_id: track["id"]
                    .as_str()
                    .ok_or("Cannot get track id")?
                    .to_owned(),
                track_name: track["name"]
                    .as_str()
                    .ok_or("Cannot get track name")?
                    .to_owned(),
                artist_name: track["artists"]
                    .as_array()
                    .ok_or("Cannot get track artists")?
                    .iter()
                    .map(|artist| artist["name"].as_str().unwrap_or("Unknown artist"))
                    .collect::<Vec<_>>()
                    .join(" - "),
                track_duration: track["duration_ms"]
                    .as_i64()
                    .ok_or("Cannot get track duration")?,
            })
        }

        Ok(tracks)
    }

    // https://developer.spotify.com/documentation/web-api/reference/add-to-queue
    pub async fn add_track_to_queue(&self, track_id: String) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send add to queue request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch add to queue: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback
    pub async fn play_resume(&self) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send play resume request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch play resume: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/pause-a-users-playback
    pub async fn pause(&self) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send pause request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch pause: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/skip-users-playback-to-previous-track
    pub async fn skip_previous(&self) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send skip to previous request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch skip to previous: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/skip-users-playback-to-next-track
    pub async fn skip_next(&self) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send skip to next request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch skip to next: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/seek-to-position-in-currently-playing-track
    pub async fn seek_to_ms(&self, ms: u64) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to send seek to pos request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch seek to pos: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/set-volume-for-users-playback
    pub async fn set_volume(&self, volume: u8) -> Result<(), String> {
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
            .map_err(|err| format!("Failed to set volume request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch set volume: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        Ok(())
    }

    pub async fn get_my_id(&self) -> Result<String, String> {
        let res = self
            .client
            .get("https://api.spotify.com/v1/me")
            .header(
                "Authorization",
                format!("Bearer {}", self.tokens.access_token),
            )
            .send()
            .await
            .map_err(|err| format!("Failed to send Spotify user info request: {err}"))?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to fetch Spotify user info: ({}) {:?}",
                res.status(),
                res.text().await.unwrap()
            ));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|err| format!("Failed to parse Spotify user info json result: {err}"))?;

        Ok(body["id"].as_str().unwrap().to_owned())
    }

    pub async fn create_playlists(&self, playlists: Vec<SpotifyPlaylist>) -> Result<(), String> {
        let id = self.get_my_id().await?;

        for playlist in playlists {
            let res = self
                .client
                .post(format!("https://api.spotify.com/v1/users/{id}/playlists"))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.tokens.access_token),
                )
                .json(&json!({
                    "name": playlist.title,
                    "description": "",
                    "public": false
                }))
                .send()
                .await
                .map_err(|err| {
                    format!("Couldn't send Spotify post resquest to create playlist {err}")
                })?;

            if !res.status().is_success() {
                return Err(format!(
                    "Failed to create Spotify playlist: ({}) {:?}",
                    res.status(),
                    res.text().await.unwrap()
                ));
            }

            let body: serde_json::Value = res
                .json()
                .await
                .map_err(|err| format!("Failed to parse Spotify playlist json result: {err}"))?;

            let playlist_id = body["id"].as_str().unwrap().to_owned();

            let uris = playlist
                .tracks
                .iter()
                .map(|t| format!("spotify:track:{}", t.track_id))
                .collect::<Vec<String>>();

            let res = self
                .client
                .post(format!(
                    "https://api.spotify.com/v1/playlists/{playlist_id}/tracks",
                ))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.tokens.access_token),
                )
                .json(&json!({ "uris": uris }))
                .send()
                .await
                .map_err(|err| {
                    format!(
                        "Couldn't send Spotify post resquest to add tracks to playlist id: {} {}",
                        playlist_id, err
                    )
                })?;

            if !res.status().is_success() {
                return Err(format!(
                    "Failed to add tracks to Spotify playlist id: {} ({}) {:?}",
                    playlist_id,
                    res.status(),
                    res.text().await.unwrap()
                ));
            }
        }
        Ok(())
    }
}
