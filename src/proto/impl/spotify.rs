use crate::proto;
use crate::sharify::spotify;
use crate::sharify::spotify_web_utils;

impl From<spotify::SpotifyError> for proto::cmd::command_response::Type {
    fn from(err: spotify::SpotifyError) -> Self {
        match err {
            spotify::SpotifyError::Generic(error) => Self::GenericError(error),
            spotify::SpotifyError::RateLimited(time) => Self::SpotifyRateLimited(time),
        }
    }
}

impl From<spotify_web_utils::SpotifyCurrentPlaybackOutput> for proto::spotify::PlaybackState {
    fn from(state: spotify_web_utils::SpotifyCurrentPlaybackOutput) -> Self {
        Self {
            device_id: state.device_id,
            device_volume: state.device_volume as _,
            shuffle: state.shuffle,
            progress_ms: state.progress_ms,
            duration_ms: state.duration_ms,
            is_playing: state.is_playing,
            track_id: state.track_id,
            track_name: state.track_name,
            artist_name: state.artist_name,
            album_image_src: state.album_image_src,
        }
    }
}

impl From<proto::spotify::Track> for spotify_web_utils::SpotifyTrack {
    fn from(track: proto::spotify::Track) -> Self {
        Self {
            track_id: track.track_id,
            track_name: track.track_name,
            artist_name: track.artist_name,
            track_duration: track.track_duration,
        }
    }
}

impl From<spotify_web_utils::SpotifyTrack> for proto::spotify::Track {
    fn from(track: spotify_web_utils::SpotifyTrack) -> Self {
        Self {
            track_id: track.track_id,
            track_name: track.track_name,
            artist_name: track.artist_name,
            track_duration: track.track_duration,
        }
    }
}

impl From<proto::spotify::TrackArray> for spotify_web_utils::SpotifyTackArray {
    fn from(tracks: proto::spotify::TrackArray) -> Self {
        tracks.tracks.into_iter().map(Into::into).collect()
    }
}

impl From<spotify_web_utils::SpotifyTackArray> for proto::spotify::TrackArray {
    fn from(tracks: spotify_web_utils::SpotifyTackArray) -> Self {
        Self {
            tracks: tracks.into_iter().map(Into::into).collect(),
        }
    }
}
