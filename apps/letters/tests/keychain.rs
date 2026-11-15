//! Sign-ins kept by a keychain, which is one in memory here: a test writes in nobody's own.

use std::sync::LazyLock;

use letters::keychain::{Keychain, Unkept};

/// The one keychain of this run, as an app has one.
static KEYCHAIN: LazyLock<Keychain> = LazyLock::new(|| Keychain::in_memory().unwrap());

#[test]
fn a_sign_in_is_kept_for_its_server_until_it_is_forgotten() {
    let (home, work) = ("https://letters.home.example", "https://letters.work.example");
    assert_eq!(KEYCHAIN.kept(home), Ok(None), "nobody has signed in yet");

    KEYCHAIN.keep(home, "first").unwrap();
    KEYCHAIN.keep(work, "another").unwrap();
    KEYCHAIN.keep(home, "renewed").unwrap();
    assert_eq!(KEYCHAIN.kept(home).unwrap().as_deref(), Some("renewed"), "a new sign-in takes the old one's place");
    assert_eq!(KEYCHAIN.kept(work).unwrap().as_deref(), Some("another"), "each server has its own");

    KEYCHAIN.forget(home).unwrap();
    assert_eq!(KEYCHAIN.kept(home), Ok(None));
    assert_eq!(KEYCHAIN.forget(home), Ok(()), "signing out twice is no failure");
    assert_eq!(KEYCHAIN.kept(work).unwrap().as_deref(), Some("another"));
}

#[test]
fn a_keychain_that_fails_says_nothing_of_what_it_kept() {
    let kept = "a-sign-in-nobody-should-read";
    // (what the keychain said, what the person is told of it)
    let cases = [
        (keyring_core::Error::NoDefaultStore, Unkept::NoKeychain),
        (keyring_core::Error::NoStorageAccess("locked".into()), Unkept::Locked),
        (keyring_core::Error::BadEncoding(kept.as_bytes().to_vec()), Unkept::Failed(String::new())),
        (keyring_core::Error::BadDataFormat(kept.as_bytes().to_vec(), kept.into()), Unkept::Failed(String::new())),
    ];
    for (error, expected) in cases {
        let said = Unkept::from(error);

        assert_eq!(std::mem::discriminant(&said), std::mem::discriminant(&expected), "{said:?}");
        assert!(!format!("{said} {said:?}").contains(kept), "{said:?}");
    }
}
