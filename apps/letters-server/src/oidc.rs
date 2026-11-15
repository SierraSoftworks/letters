//! The identity provider: OpenID Connect's authorization code flow with PKCE, from the client's side.
//!
//! [`Provider::begin`] says where to send a browser, and [`Provider::finish`] trades the code it comes
//! back with for an ID token and reads the person and their groups from it. The provider describes itself
//! at a well-known address under its issuer, so the settings name the issuer and nothing else of it.
//!
//! Every call here waits on the network, so it runs where blocking is allowed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use letters_api::{Code, Person, paths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use crate::config::{Config, Oidc, local_or_secure};
use crate::error::{Answer, Refusal};
use crate::ids::{base64, base64url, secret};
use crate::log;

/// How long the provider's description of itself is taken as read.
const DISCOVERY_LIFE: u64 = 60 * 60;
/// The provider's keys are not asked for more often than this, whatever key a token names.
const KEYS_REST: u64 = 60;
/// How far the provider's clock may be from the server's, in seconds.
const LEEWAY: u64 = 60;
/// How long the provider has to answer.
const PATIENCE: Duration = Duration::from_secs(10);
/// The longest answer read from the provider.
const LARGEST: u64 = 1 << 20;
/// The ways a provider may sign a token: with a key pair, never with a secret both sides hold.
const SIGNED_WITH: [Algorithm; 9] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// What the provider says of itself.
#[derive(Clone, Debug, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Vec<String>,
}

/// The provider as it was last read.
struct Known {
    discovery: Discovery,
    read: u64,
    keys: JwkSet,
    keys_read: u64,
}

/// What the provider answers a code with.
#[derive(Deserialize)]
struct Tokens {
    id_token: String,
    #[serde(default)]
    access_token: Option<String>,
}

/// What one sign-in sent to the provider, and must be shown again when its browser comes back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    /// Ties the browser that comes back to the one that left.
    pub state: String,
    /// Comes back inside the ID token, so that a token made for another sign-in is no use to this one.
    pub nonce: String,
    /// The PKCE secret whose hash the provider was shown, so that a code is no use to whoever overhears it.
    pub verifier: String,
}

pub struct Provider {
    oidc: Oidc,
    /// Where the provider sends a browser back to.
    redirect: String,
    known: Mutex<Option<Arc<Known>>>,
    agent: Agent,
}

impl Provider {
    pub fn new(config: &Config) -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(PATIENCE))
            // The provider's addresses are the ones it names, and nowhere it sends the server on to.
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .user_agent(concat!("letters-server/", env!("CARGO_PKG_VERSION")))
            // A provider on the owner's own network is vouched for by the certificates their system trusts.
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .build();
        Self {
            oidc: config.auth.oidc.clone(),
            redirect: format!("{}{}", config.server.url, paths::CALLBACK_PAGE),
            known: Mutex::new(None),
            agent: agent.into(),
        }
    }

    /// Starts a sign-in: where to send the browser, and what to keep until it is back.
    pub fn begin(&self, now: u64) -> Answer<(String, Sent)> {
        let known = self.known(now)?;
        let sent = Sent { state: secret(), nonce: secret(), verifier: secret() };
        let challenge = base64url(&Sha256::digest(sent.verifier.as_bytes()));
        let scopes = self.oidc.scopes.join(" ");
        let query = form(&[
            ("response_type", "code"),
            ("client_id", &self.oidc.client_id),
            ("redirect_uri", &self.redirect),
            ("scope", &scopes),
            ("state", &sent.state),
            ("nonce", &sent.nonce),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        let endpoint = &known.discovery.authorization_endpoint;
        let join = if endpoint.contains('?') { '&' } else { '?' };
        Ok((format!("{endpoint}{join}{query}"), sent))
    }

    /// Ends a sign-in: trades the code the browser came back with, and says who the provider vouches for
    /// and which groups it puts them in.
    pub fn finish(&self, code: &str, sent: &Sent, now: u64) -> Answer<(Person, Vec<String>)> {
        let known = self.known(now)?;
        let tokens = self.trade(&known, code, &sent.verifier)?;
        let mut claims = self.read(known, &tokens.id_token, &sent.nonce, now)?;
        // A provider may keep some claims out of the token and tell them when asked.
        let lacks = |claim: &String| !claims.contains_key(claim);
        if lacks(&self.oidc.username_claim) || lacks(&self.oidc.groups_claim) {
            let known = self.known(now)?;
            if let (Some(endpoint), Some(access)) = (&known.discovery.userinfo_endpoint, &tokens.access_token) {
                let told = self.ask(
                    "userinfo",
                    self.agent.get(endpoint).header("Authorization", format!("Bearer {access}")).call(),
                );
                match told.and_then(object) {
                    // What is said of another subject is not about this person.
                    Ok(told) if told.get("sub") == claims.get("sub") => {
                        for (claim, value) in told {
                            claims.entry(claim).or_insert(value);
                        }
                    }
                    Ok(_) => {
                        log("the provider's userinfo was about another subject than its token, and was left unread")
                    }
                    Err(_) => {}
                }
            }
        }
        person(&self.oidc, &claims)
    }

    /// The provider as it describes itself, read again once an hour. What was read before stands while
    /// the provider cannot be asked.
    fn known(&self, now: u64) -> Answer<Arc<Known>> {
        // Held while asking, so that sign-ins that start together ask once.
        let mut held = self.known.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(known) = &*held
            && now < known.read + DISCOVERY_LIFE
        {
            return Ok(known.clone());
        }
        match self.discover(now) {
            Ok(known) => Ok(held.insert(Arc::new(known)).clone()),
            Err(refusal) => held.clone().ok_or(refusal),
        }
    }

    fn discover(&self, now: u64) -> Answer<Known> {
        let issuer = &self.oidc.issuer;
        let address = format!("{issuer}/.well-known/openid-configuration");
        let discovery: Discovery = self.ask("discovery", self.agent.get(&address).call()).and_then(shaped)?;
        // A description of another issuer is no description of this one.
        if discovery.issuer.trim_end_matches('/') != issuer {
            log(&format!("{address} describes the issuer \"{}\", not \"{issuer}\"", discovery.issuer));
            return Err(unreachable());
        }
        let mut endpoints = vec![&discovery.authorization_endpoint, &discovery.token_endpoint, &discovery.jwks_uri];
        endpoints.extend(&discovery.userinfo_endpoint);
        if let Some(open) = endpoints.into_iter().find(|endpoint| !local_or_secure(endpoint)) {
            log(&format!("the provider names \"{open}\", which is without TLS"));
            return Err(unreachable());
        }
        let keys = self.keys(&discovery)?;
        Ok(Known { discovery, read: now, keys, keys_read: now })
    }

    fn keys(&self, discovery: &Discovery) -> Answer<JwkSet> {
        self.ask("keys", self.agent.get(&discovery.jwks_uri).call()).and_then(shaped)
    }

    /// The provider with its keys read again, for a token signed with a key that is not among them. A
    /// provider changes its keys now and then, and says so nowhere else.
    fn rekeyed(&self, now: u64) -> Answer<Arc<Known>> {
        let mut held = self.known.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(known) = held.clone() else { return Err(unreachable()) };
        if now < known.keys_read + KEYS_REST {
            return Ok(known);
        }
        let keys = self.keys(&known.discovery)?;
        let known = Known { discovery: known.discovery.clone(), read: known.read, keys, keys_read: now };
        Ok(held.insert(Arc::new(known)).clone())
    }

    /// Trades a code for its tokens, with the PKCE secret and whatever the provider knows the client by.
    fn trade(&self, known: &Known, code: &str, verifier: &str) -> Answer<Tokens> {
        let mut fields = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.redirect),
            ("code_verifier", verifier),
        ];
        let endpoint = &known.discovery.token_endpoint;
        let mut request = self.agent.post(endpoint).header("Accept", "application/json");
        let methods = &known.discovery.token_endpoint_auth_methods_supported;
        let takes = |method: &str| methods.iter().any(|supported| supported == method);
        match &self.oidc.client_secret {
            // In the header, unless the provider says it only reads the form.
            Some(secret) if methods.is_empty() || takes("client_secret_basic") || !takes("client_secret_post") => {
                let pair = format!("{}:{}", paths::segment(&self.oidc.client_id), paths::segment(secret));
                request = request.header("Authorization", format!("Basic {}", base64(pair.as_bytes())));
            }
            Some(secret) => fields.extend([("client_id", self.oidc.client_id.as_str()), ("client_secret", secret)]),
            None => fields.push(("client_id", &self.oidc.client_id)),
        }
        let answer = request.header("Content-Type", "application/x-www-form-urlencoded").send(form(&fields));
        self.ask("token", answer).and_then(shaped)
    }

    /// Reads an ID token: signed by the provider, made for this client and this sign-in, and not run out.
    fn read(&self, mut known: Arc<Known>, token: &str, nonce: &str, now: u64) -> Answer<Map<String, Value>> {
        let refused = |why: &str| {
            log(&format!("the provider's ID token was refused: {why}"));
            Refusal::new(Code::Upstream, "The identity provider's answer could not be trusted. Try to sign in again.")
        };
        let header = decode_header(token).map_err(|error| refused(&error.to_string()))?;
        if !SIGNED_WITH.contains(&header.alg) {
            return Err(refused("it is not signed with a key pair"));
        }
        let key = |known: &Known| -> Option<Jwk> {
            match &header.kid {
                Some(kid) => known.keys.find(kid).cloned(),
                // A provider with one key need not name it.
                None => known.keys.keys.first().filter(|_| known.keys.keys.len() == 1).cloned(),
            }
        };
        let jwk = match key(&known) {
            Some(jwk) => jwk,
            None => {
                known = self.rekeyed(now)?;
                key(&known).ok_or_else(|| refused("it is signed with a key the provider does not list"))?
            }
        };
        let key = DecodingKey::from_jwk(&jwk).map_err(|error| refused(&error.to_string()))?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&known.discovery.issuer]);
        validation.set_audience(&[&self.oidc.client_id]);
        validation.set_required_spec_claims(&["iss", "aud", "exp", "sub"]);
        // The time is the server's own to tell, so it is held to below.
        validation.validate_exp = false;
        let claims: Map<String, Value> =
            decode(token, &key, &validation).map_err(|error| refused(&error.to_string()))?.claims;
        let expires = claims.get("exp").and_then(Value::as_f64).unwrap_or(0.0);
        if expires + LEEWAY as f64 <= now as f64 {
            return Err(refused("it has run out"));
        }
        if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return Err(refused("it was made for another sign-in"));
        }
        // A token for several clients says which one asked for it.
        if claims.get("azp").is_some_and(|party| party.as_str() != Some(&self.oidc.client_id)) {
            return Err(refused("it was made for another client"));
        }
        Ok(claims)
    }

    /// Reads the provider's answer as JSON. What went wrong is for the log, and the person is told only
    /// that the provider failed.
    fn ask(&self, what: &str, answer: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Answer<Value> {
        let failed = |why: String| {
            log(&format!("the identity provider's {what} failed: {why}"));
            unreachable()
        };
        let mut answer = answer.map_err(|error| failed(error.to_string()))?;
        let status = answer.status();
        let text = answer.body_mut().with_config().limit(LARGEST).read_to_string();
        let text = text.map_err(|error| failed(error.to_string()))?;
        if !status.is_success() {
            // What a provider says of a refusal is short: an error's name and a sentence.
            let said: String = text.chars().filter(|c| !c.is_control()).take(300).collect();
            return Err(failed(format!("{status}: {said}")));
        }
        serde_json::from_str(&text).map_err(|error| failed(format!("its answer is not JSON: {error}")))
    }
}

fn unreachable() -> Refusal {
    Refusal::new(Code::Upstream, "The identity provider could not be asked. Try again in a moment.")
}

fn shaped<T: serde::de::DeserializeOwned>(value: Value) -> Answer<T> {
    serde_json::from_value(value).map_err(|error| {
        log(&format!("the identity provider's answer is not what OpenID Connect lays down: {error}"));
        unreachable()
    })
}

fn object(value: Value) -> Answer<Map<String, Value>> {
    shaped(value)
}

/// Who the claims are about, and their groups.
fn person(oidc: &Oidc, claims: &Map<String, Value>) -> Answer<(Person, Vec<String>)> {
    let text = |claim: &str| claims.get(claim).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    let lacking = |claim: &str| {
        log(&format!("the provider told no \"{claim}\" for someone who signed in"));
        Refusal::forbidden(format!(
            "The identity provider did not say your \"{claim}\", which this server knows people by. Ask whoever runs \
             this server."
        ))
    };
    let id = text("sub").ok_or_else(|| lacking("sub"))?;
    let username = text(&oidc.username_claim).ok_or_else(|| lacking(&oidc.username_claim))?;
    let name = text(&oidc.name_claim).unwrap_or(username);
    let groups = match claims.get(&oidc.groups_claim) {
        Some(Value::Array(groups)) => groups.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        Some(Value::String(group)) => vec![group.clone()],
        _ => Vec::new(),
    };
    Ok((Person { id: id.into(), username: username.into(), name: name.into() }, groups))
}

/// Fields as a form or a query writes them.
pub fn form(fields: &[(&str, &str)]) -> String {
    let pairs = fields.iter().map(|(name, value)| format!("{}={}", paths::segment(name), paths::segment(value)));
    pairs.collect::<Vec<_>>().join("&")
}
