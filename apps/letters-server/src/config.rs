//! The server's settings, read once from a TOML file when it starts.
//!
//! A setting that is not known is an error and not ignored: a misspelt `editors` would otherwise leave a
//! space open to nobody, or a guard off, without a word.

use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use letters_trust::Source;
use serde::Deserialize;

/// What stands in a personal space's path for the name its person signs in with.
pub const USERNAME: &str = "{username}";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub auth: Auth,
    #[serde(default, rename = "space")]
    pub spaces: Vec<Space>,
    #[serde(default)]
    pub trust: Trust,
    #[serde(default)]
    pub links: Links,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// The server's name, as its people know it.
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    /// The address people reach the server at, such as `https://letters.example.ts.net`. The redirect
    /// address registered with the identity provider is this with `/auth/callback`.
    pub url: String,
    /// Where the server keeps what files cannot hold. On local disk.
    pub state: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub oidc: Oidc,
    /// How long a sign-in lasts. Who is in which group is read when a person signs in, so this is also how
    /// long a person taken out of a group keeps what it gave them.
    #[serde(default = "default_session_days")]
    pub session_days: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    /// The client's secret, or a file that holds it. A provider that asks for none gets none.
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_secret_file: Option<PathBuf>,
    /// The claim a person's space is named by.
    #[serde(default = "default_username_claim")]
    pub username_claim: String,
    /// The claim a person is shown by.
    #[serde(default = "default_name_claim")]
    pub name_claim: String,
    /// The claim that lists a person's groups.
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

/// A named folder. One whose path holds [`USERNAME`] is each person's own, made when they first sign in.
/// Any other is shared with the groups it lists.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Space {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub editors: Vec<String>,
    #[serde(default)]
    pub commenters: Vec<String>,
    #[serde(default)]
    pub readers: Vec<String>,
}

impl Space {
    /// What names the space in an address: its name in lower case, with a `-` for whatever is not a letter
    /// or a digit.
    pub fn id(&self) -> String {
        let mut id = String::new();
        for c in self.name.chars().flat_map(char::to_lowercase) {
            match c.is_alphanumeric() {
                true => id.push(c),
                false if !id.ends_with('-') && !id.is_empty() => id.push('-'),
                false => {}
            }
        }
        id.trim_end_matches('-').to_owned()
    }

    pub fn personal(&self) -> bool {
        self.path.contains(USERNAME)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trust {
    /// Where the server itself may fetch pictures and link previews from, for everyone.
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Links {
    /// Whether documents can be shared by link. Off unless the server's owner says so.
    #[serde(default)]
    pub enabled: bool,
}

fn default_name() -> String {
    "Letters".into()
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8080))
}

fn default_session_days() -> u32 {
    7
}

fn default_username_claim() -> String {
    "preferred_username".into()
}

fn default_name_claim() -> String {
    "name".into()
}

fn default_groups_claim() -> String {
    "groups".into()
}

fn default_scopes() -> Vec<String> {
    ["openid", "profile", "email"].map(String::from).into()
}

/// Why the settings cannot be used, in words for whoever wrote them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads the settings and holds them to what the server can run with.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut config: Self = toml::from_str(text).map_err(|error| ConfigError(error.to_string()))?;
        config.server.url = origin(&config.server.url, "server.url")?;
        config.auth.oidc.issuer = issuer(&config.auth.oidc.issuer)?;
        let oidc = &config.auth.oidc;
        if oidc.client_id.trim().is_empty() {
            return Err(ConfigError("auth.oidc.client_id is empty".into()));
        }
        if oidc.client_secret.is_some() && oidc.client_secret_file.is_some() {
            return Err(ConfigError("auth.oidc has both client_secret and client_secret_file: keep one".into()));
        }
        if !oidc.scopes.iter().any(|scope| scope == "openid") {
            return Err(ConfigError("auth.oidc.scopes must hold \"openid\"".into()));
        }
        if config.auth.session_days == 0 {
            return Err(ConfigError("auth.session_days must be at least 1".into()));
        }
        if config.spaces.is_empty() {
            return Err(ConfigError("there is no [[space]], so nobody would have anywhere to keep a document".into()));
        }
        let mut ids = HashSet::new();
        for space in &config.spaces {
            let (name, id) = (&space.name, space.id());
            if id.is_empty() {
                return Err(ConfigError(format!("the space \"{name}\" needs a name with a letter or a digit in it")));
            }
            if !ids.insert(id) {
                return Err(ConfigError(format!(
                    "two spaces are called \"{name}\", or names that differ only in case and signs"
                )));
            }
            if !space.path.starts_with('/') {
                return Err(ConfigError(format!("the path of the space \"{name}\" must start at the root, with a /")));
            }
            let plain = space.path.replace(USERNAME, "");
            if plain.contains(['{', '}']) {
                return Err(ConfigError(format!(
                    "the path of the space \"{name}\" may hold {USERNAME} and nothing else in braces"
                )));
            }
            let groups = space.editors.len() + space.commenters.len() + space.readers.len();
            match (space.personal(), groups) {
                (true, 1..) => {
                    return Err(ConfigError(format!(
                        "the space \"{name}\" is each person's own, so it takes no editors, commenters or readers"
                    )));
                }
                (false, 0) => {
                    return Err(ConfigError(format!(
                        "the space \"{name}\" names no editors, commenters or readers, so nobody would reach it"
                    )));
                }
                _ => {}
            }
        }
        config.sources()?;
        Ok(config)
    }

    /// Reads the client's secret from its file, when the settings name one. The file may end in a line
    /// break, as files do.
    pub fn read_secrets(&mut self) -> Result<(), ConfigError> {
        let oidc = &mut self.auth.oidc;
        if let Some(file) = oidc.client_secret_file.take() {
            let secret = std::fs::read_to_string(&file).map_err(|error| {
                ConfigError(format!(
                    "auth.oidc.client_secret_file is \"{}\", which cannot be read: {error}",
                    file.display()
                ))
            })?;
            oidc.client_secret = Some(secret.trim().to_owned());
        }
        if oidc.client_secret.as_deref().is_some_and(str::is_empty) {
            return Err(ConfigError(
                "auth.oidc has a client secret with nothing in it: leave it out, or fill it in".into(),
            ));
        }
        Ok(())
    }

    /// The sources the server itself fetches from.
    pub fn sources(&self) -> Result<Vec<Source>, ConfigError> {
        let read = |text: &String| {
            Source::parse(text)
                .map_err(|error| ConfigError(format!("trust.sources: \"{text}\" is not a source: {error}")))
        };
        self.trust.sources.iter().map(read).collect()
    }
}

/// Whether `url` is reached over TLS, or on this machine, which alone may go without. `None` when it is
/// no `http` address at all.
fn secure(url: &str) -> Option<bool> {
    let (secure, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (true, rest),
        (_, Some(rest)) => (false, rest),
        _ => return None,
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once(':').filter(|(_, port)| port.parse::<u16>().is_ok()).map_or(host, |(host, _)| host);
    Some(secure || matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
}

/// Whether `url` is an address the server may send a secret to: over TLS, or on this machine.
pub fn local_or_secure(url: &str) -> bool {
    secure(url) == Some(true)
}

/// Holds `url` to `scheme://host[:port]` and hands it back without a slash at its end. Only a server on
/// this machine may go without TLS.
fn origin(url: &str, setting: &str) -> Result<String, ConfigError> {
    let url = url.trim().trim_end_matches('/');
    let bad = |why: &str| ConfigError(format!("{setting} is \"{url}\", {why}"));
    let secure = secure(url).ok_or_else(|| bad("which is not an address that starts with https://"))?;
    let rest = url.split_once("://").map_or("", |(_, rest)| rest);
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return Err(bad("which must be a host and no more, such as https://letters.example.com"));
    }
    if !secure {
        return Err(bad("which is without TLS: only a server on this machine may be reached over http://"));
    }
    Ok(url.to_owned())
}

/// Holds an issuer to an address as [`origin`] does a server's, but lets it live under a path, as an
/// issuer may.
fn issuer(url: &str) -> Result<String, ConfigError> {
    let url = url.trim().trim_end_matches('/');
    let host = url.find("://").map_or(url.len(), |at| at + 3);
    let path = url[host..].find('/').map_or(url.len(), |at| host + at);
    origin(&url[..path], "auth.oidc.issuer")?;
    if url[path..].contains(['?', '#', ' ']) {
        return Err(ConfigError(format!("auth.oidc.issuer is \"{url}\", which must be an address and no more")));
    }
    Ok(url.to_owned())
}
