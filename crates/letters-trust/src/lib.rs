//! Safe mode and trusted sources.
//!
//! A document from outside opens in safe mode. It reads and edits as normal, and nothing in it reaches the
//! network or the user's other documents until the user trusts it.
//!
//! - [`TrustStore`] keeps what the user trusts (documents, folders and image sources) as TOML in a file the
//!   shell lends it. Trust lives outside the document, so a file cannot declare itself trusted.
//! - [`Trust::gate`] decides how one open document is treated, and its [`Gate`] answers for each remote
//!   image, link and merge.
//! - [`ImageFetch`] guards one download: HTTPS only, no private addresses but those of a trusted source, and
//!   limits on size and type.
//!
//! Nothing here opens a file or a socket: the shell does both and asks before each step.

mod address;
mod fetch;
mod file;
mod gate;
mod path;
mod source;
mod store;
mod url;

pub use address::is_public;
pub use fetch::{FetchError, ImageFetch, ImageType, Limits};
pub use file::Trust;
pub use gate::{Gate, ImageDecision, LinkDecision, MergeDecision, Mode, Origin};
pub use source::Source;
pub use store::{Store, TrustError, TrustStore};
pub use url::{Host, HttpsUrl, UrlError};
