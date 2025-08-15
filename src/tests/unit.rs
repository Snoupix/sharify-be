use regex::Regex;

use crate::sharify::utils::*;

const LENGTH: usize = 15;
const DUMMY_EMAILS: [&str; 6] = [
    "test@hotmail.com",
    "dummy-email@gmail.com",
    "invalid\\/email@wrong,;^$.chars",
    "smol@email.io",
    "i-lack_ideas_for-this-one@gmail.com",
    "i_am_bond_james_bond-007@mail.uk",
];

fn are_emails_alike(a: String, b: String) -> bool {
    a.chars().zip(b.chars()).enumerate().fold(
        true,
        |b, (_, (char1, char2))| if b { char1 == char2 } else { b },
    )
}

// Email & HEX UUID conversions
#[test]
fn converts_email_to_valid_uuid() {
    let reg = Regex::new(&format!("(:?(\\d|[A-F]){{4}}:?){{{LENGTH}}}")).unwrap();

    for email in DUMMY_EMAILS {
        let hex = encode_user_email(email.to_owned(), LENGTH);

        assert!(reg.is_match(&hex));
    }
}

#[test]
fn converts_uuid_to_string() {
    for email in DUMMY_EMAILS {
        let hex = encode_user_email(email.to_owned(), LENGTH);

        let hex_in_str = decode_user_email(&hex);

        if email_contains_invalid_chars(email.to_owned()) {
            assert!(!are_emails_alike(email.to_owned(), hex_in_str));
            continue;
        }

        assert!(are_emails_alike(email.to_owned(), hex_in_str));
    }
}

#[test]
fn converts_uuid_to_initial_email() {
    let length = DUMMY_EMAILS.iter().fold(
        0,
        |i, &email| if email.len() > i { email.len() + 1 } else { i },
    );

    for email in DUMMY_EMAILS {
        if email_contains_invalid_chars(email.to_owned()) {
            continue;
        }

        let hex = encode_user_email(email.to_owned(), length);

        let res = hex_uuid_to_valid_email(hex, email.len());

        assert!(res.is_some_and(|e| e == email));
    }
}
