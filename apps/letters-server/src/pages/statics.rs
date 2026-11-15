//! What the server's pages load: yewi's stylesheet and script, the few rules of the server's own, and the
//! fonts. They are part of the program, and are served under an address that names their contents, so a
//! browser fetches each once and keeps it until the server is another.

use std::sync::OnceLock;

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::ids::hex;

const CSS: &str = "text/css; charset=utf-8";
/// The rules of the server's own pages, on top of yewi's.
const OWN: &str = include_str!("letters.css");
/// Where the fonts a document is set in are, under [`Statics::base`], by their names in
/// [`letters_fonts::FILES`].
pub const FACES: &str = "faces";

struct File {
    name: String,
    kind: &'static str,
    bytes: Bytes,
}

pub struct Statics {
    /// Where the files are: an address with no slash at its end.
    pub base: String,
    files: Vec<File>,
}

/// The files, read once.
pub fn statics() -> &'static Statics {
    static STATICS: OnceLock<Statics> = OnceLock::new();
    STATICS.get_or_init(|| {
        let file =
            |name: &str, kind, bytes: &'static [u8]| File { name: name.into(), kind, bytes: Bytes::from_static(bytes) };
        let mut files = vec![
            file("yewi.css", CSS, yewi::STYLESHEET.as_bytes()),
            file("yewi.js", "text/javascript; charset=utf-8", yewi::BEHAVIOURS.as_bytes()),
            file("letters.css", CSS, OWN.as_bytes()),
            // The fonts of the pages themselves. The rules name each file beside them.
            file("fonts/fonts.css", CSS, yewi::fonts::RELATIVE_CSS.as_bytes()),
        ];
        files.extend(
            yewi::fonts::FILES
                .iter()
                .map(|font| file(&format!("fonts/{}", font.name), yewi::fonts::CONTENT_TYPE, font.bytes)),
        );
        // The fonts documents are set in, whole: a document may be in any script they hold.
        let faces = letters_fonts::FILES.iter().zip(letters_fonts::bundled());
        files.extend(faces.map(|(name, font)| File {
            name: format!("{FACES}/{name}"),
            kind: "font/ttf",
            bytes: font.data.to_vec().into(),
        }));
        let mut contents = Sha256::new();
        for file in &files {
            contents.update(file.name.as_bytes());
            contents.update((file.bytes.len() as u64).to_be_bytes());
            contents.update(&file.bytes);
        }
        Statics { base: format!("/static/{}", &hex(&contents.finalize())[..12]), files }
    })
}

/// `GET /static/{version}/{*file}`.
pub async fn file(Path((version, name)): Path<(String, String)>) -> Response {
    let statics = statics();
    let found = statics.files.iter().find(|file| file.name == name);
    match found.filter(|_| statics.base.ends_with(&format!("/{version}"))) {
        Some(file) => {
            let headers = [
                (CONTENT_TYPE, file.kind),
                // The address changes when the bytes do.
                (CACHE_CONTROL, "public, max-age=31536000, immutable"),
                (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ];
            (headers, file.bytes.clone()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
