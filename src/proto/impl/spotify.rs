use crate::proto;
use crate::sharify::spotify_web_utils;

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
