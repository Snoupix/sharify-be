use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE;
use rand::distr::Alphanumeric;
use rand::{Rng, rng};
use sha2::{Digest, Sha256};

use super::room::{MAX_EMAIL_CHAR, MIN_EMAIL_CHAR, RoomUserID};

#[macro_export]
macro_rules! match_flags {
    ($flags:expr, $([$flag:expr; $fut:expr]),+; [$placeholder:ident; $fallback:expr]) => {
        match $flags {
            $(f if f & $flag == $flag => $fut.await?),+,
            $placeholder => $fallback,
        }
    };
}

pub type SpotifyFetchT = u8;

static __COMPTIME_ASSERTIONS: () = {
    assert!((MIN_EMAIL_CHAR as u8) < (MAX_EMAIL_CHAR as u8));
};

pub const SPOTIFY_FETCH_ALL: SpotifyFetchT = SPOTIFY_FETCH_PLAYBACK | SPOTIFY_FETCH_TRACKS_Q;
pub const SPOTIFY_FETCH_PLAYBACK: SpotifyFetchT = 1 << 0;
pub const SPOTIFY_FETCH_TRACKS_Q: SpotifyFetchT = 1 << 1;

pub fn generate_code_verifier() -> String {
    rng()
        .sample_iter(&Alphanumeric)
        .take(128)
        .map(char::from)
        .collect()
}

pub fn generate_code_challenge(code_verifier: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier);
    let hashed = hasher.finalize();
    BASE64_URL_SAFE
        .encode(hashed)
        .replace('=', "")
        .replace('+', "-")
        .replace('/', "_")
}

pub fn get_authorized_bytes() -> Vec<char> {
    std::iter::once('0')
        .chain(MIN_EMAIL_CHAR..MAX_EMAIL_CHAR)
        .collect()
}

pub fn encode_user_email(email: String, uuid_len: usize) -> String {
    if email.trim() == "" {
        return "".into();
    }

    let authorized_bytes = get_authorized_bytes();

    let mut hex_values = Vec::with_capacity(uuid_len);
    let mut split = email.chars();

    for i in 0..email.len() {
        // Allows the last char to be handled even
        // if the index is odd, the left byte will be a 0
        // so the email can be recontructed from the UUID
        if (i & 1) == 1 && i != email.len() - 1 {
            continue;
        }

        let byte_one = split.next().unwrap_or('0');
        let byte_two = split.next().unwrap_or('0');

        if !authorized_bytes.contains(&byte_one) || !authorized_bytes.contains(&byte_two) {
            continue;
        }

        hex_values.push(format!("{:02X}{:02X}", byte_one as u8, byte_two as u8));
    }

    if hex_values.is_empty() {
        return "".into();
    }

    for i in 0.. {
        if hex_values.len() >= uuid_len {
            break;
        }

        hex_values.push(hex_values[i % hex_values.len()].clone());
    }

    hex_values.join(":")
}

pub fn decode_user_email(user_id: &RoomUserID) -> String {
    user_id.split(':').fold(String::new(), |mut res, s| {
        let (b1, b2) = (
            u8::from_str_radix(&s[0..=1], 16).unwrap(),
            u8::from_str_radix(&s[2..=3], 16).unwrap(),
        );

        res.push(b1 as char);
        res.push(b2 as char);
        res
    })
}

pub fn email_contains_invalid_chars(email: String) -> bool {
    let authorized_bytes = get_authorized_bytes();

    email.chars().any(|c| !authorized_bytes.contains(&c))
}

pub fn hex_uuid_to_valid_email(hex: String, email_len: usize) -> Option<String> {
    // HEX UUID is too small to contain the email
    if hex.replace(':', "").len() < email_len {
        return None;
    }

    let email = decode_user_email(&hex);
    if email.len() == email_len {
        return Some(email);
    }

    Some(email[0..email_len].to_owned())
}
