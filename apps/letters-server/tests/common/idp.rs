//! An identity provider on this machine: OpenID Connect, as far as a server asks for it.
//!
//! A test plays the person: it takes the address the server sends a browser to, has the provider agree
//! with [`Idp::approve`], and brings the code back to the server as the browser would.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use letters_server::ids::{base64, base64url};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{START, URL, query};

/// What the provider knows the server by.
pub const CLIENT: &str = "letters";
/// A secret with signs in it, which a header and a form each write their own way.
pub const SECRET: &str = "s3cret/with+signs & spaces";

/// An RSA key made for these tests and used nowhere else, as PKCS#1 in base64.
const RSA: &str =
    "MIIEowIBAAKCAQEA0sPtqDnsspJejHpG4Nyo/Z0kybXx4wuPlEgyGETK0VxgEBGMXyfnHM1dvXglkhoRAjC9mATuEc2juTrbSdjl\
MSLXuIhinvQsrZX7ylMcfJw15EQy+xUq1kRS2vwgeSQmolJGUlrSGbOqYrksWN6pwMpOyn1QRgjCIpEtgQy+EaZsNEMposDA6W2y\
SzDAN7G51b6JSJhtI7qUcM4BAlcX+e2jkWGnlWO/Sk+ng9UHjdtoHM/OmOheTxZwxGB+gRokXb+nE+xa90bUg371hwy4mqWCbk0y\
oTWwudhJ+sEY3uMJptxSNBZOeWBkbhWCI1A6zsx9EjxVNQ6FzCSf56S6LQIDAQABAoIBAA9pemB50+ApqeD13rxPW8rSiep3c+JZ\
aby7HBwAM8s5c7YubP2sVPnF3rFIhDQkfyfng9sGSUO4rjHmUZDF29n1pUv+KnqwekU7RCKYZJXhdQ4Qb/Xs/KlBwm6hpIeXPX1T\
LNyVfZWZjhUSsLZ9mcYvmlwI69d4iScD8g2PbqVVoaVQV/etdmZ/ejzbScNYbWWB04j8jlPK+DlwDsyvwRtlL1NnLJNTHFluWxCY\
ce0HbqlaBJTZvXLnsRhhkVfg2xzivj2Nw8lgqFkjrlPl8CR34z9vRNKhjqL78DiuR+iYvGj80psIflg+vQWqRxh4arp6M+Fr771b\
WwsTIMBPefUCgYEA/6Awpo3n3/RtwTbTmVC2hJlZdMyx1JE7OayXESnti6JP5RvSYG6R9mo6puHTxtk+I3T936/JdVRg0lzKpKMI\
ZLDVeJhvFM4qp5lUbQ6vxnD0QlKvZx4xsCQiFDCamH8r0fTFX8tPNqr3tySontjgHo8xEkTODQQ5s8y/0ieBbycCgYEA0xLspBfQ\
zvByqdrJPPvXChzdreUBia94WxVA4L3VaVhUpYRUdxO/er+Q2pVjf1cc0Bb7zJrmDn+JTJA4JtHlpKjPKa04yn0htcQvFZbQuEVc\
lEdJBgvRfVL2DV9AKhLdgCVod7ttytydIzax6uPB+QP+3RXE+MuCRGEtJpkToIsCgYBOYFFKX+wecJVOTn8mXOb5VQNmmnRT1ouw\
LUjMTGy3CXcxqgwg4QYD5aJCFK+YIne462ADu6QrQ0nEdgrz28DxDn8FUMeOO4A8jvihz4T5dRyPUUbSeSGG/i0sv5JG+4b2AvSZ\
YH7TJix5jNXejmR5OjIzhcEfoJIInNibmN1ItQKBgEfdx6jPWw86my/uCFq1PLB6x5evw2hH0GjiwaZDvGgT5S498ixhEu8OTpm+\
B5SR6oOHb9BoZTBGiE8DQ7sj+FaqC28bRZzcIiArXgSxxvsW0sDBzME8QOW024YOCmDI75X0Jq+tp2JM30vgEY/yYFcS3zITeW9k\
i0QQ+jiMIjylAoGBAMBOaIwnXNkqCfA3Lh1j7SLwCaEjwoL6ktSaSMcS+9Td9r78w/xkSBvlIfiPqBnNPxKjdxOVDyLiX39TNO/P\
jLv5dCqxd5aKvH1BCnTyrkR/0CbTMjSl/tlFm09KitulT+zPsvgh23whnSrEbIey2ejRdvoWK5GAROqxVilai0qG
";

/// What a test may have the provider do wrong, or differently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quirk {
    /// Nothing: a token as it should be.
    #[default]
    None,
    /// The token is for another client.
    OtherAudience,
    /// The token names another issuer.
    OtherIssuer,
    /// The token ran out an hour ago.
    RunOut,
    /// The token answers another sign-in.
    OtherNonce,
    /// The token is signed with a key the provider does not list.
    UnlistedKey,
    /// The token is signed with the client's secret, as whoever knows the secret could.
    SignedWithSecret,
    /// The token says a second client asked for it.
    OtherParty,
    /// The token holds the subject alone, and the rest is told at the userinfo address.
    ClaimsOnRequest,
    /// The secret is only read from the form.
    SecretInForm,
}

/// A sign-in the provider agreed to, until its code is traded.
struct Grant {
    challenge: String,
    nonce: String,
    claims: Value,
}

struct Inner {
    issuer: String,
    /// The keys the provider signs with: the last one signs, and all are listed.
    keys: Vec<(String, Algorithm, EncodingKey)>,
    grants: HashMap<String, Grant>,
    /// What the userinfo address tells, by access token.
    told: HashMap<String, Value>,
    quirk: Quirk,
    made: u32,
}

#[derive(Clone)]
pub struct Idp {
    pub issuer: String,
    inner: Arc<Mutex<Inner>>,
}

impl Idp {
    /// Starts a provider on a port of this machine.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let rsa = EncodingKey::from_rsa_der(&unbase64(RSA));
        let inner = Inner {
            issuer: issuer.clone(),
            keys: vec![("rsa-1".into(), Algorithm::RS256, rsa)],
            grants: HashMap::new(),
            told: HashMap::new(),
            quirk: Quirk::None,
            made: 0,
        };
        let idp = Self { issuer, inner: Arc::new(Mutex::new(inner)) };
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/keys", get(keys))
            .route("/token", post(token))
            .route("/userinfo", get(userinfo))
            .with_state(idp.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        idp
    }

    pub fn quirk(&self, quirk: Quirk) {
        self.inner.lock().unwrap().quirk = quirk;
    }

    /// The provider signs with a new key from now on, as providers do now and then.
    pub fn change_keys(&self) {
        let mut inner = self.inner.lock().unwrap();
        let nth = inner.keys.len() as u8;
        inner.keys.push((format!("ed-{nth}"), Algorithm::EdDSA, ed_key(nth)));
    }

    /// The person signs in at the address the server sent their browser to, as `claims` describe them.
    /// The code the browser takes back to the server.
    pub fn approve(&self, address: &str, claims: Value) -> String {
        let (endpoint, asked) = address.split_once('?').expect("the server asks with a query");
        let asked = query(asked);
        assert_eq!(endpoint, format!("{}/authorize", self.issuer));
        assert_eq!(asked["response_type"], "code");
        assert_eq!(asked["client_id"], CLIENT);
        assert_eq!(asked["redirect_uri"], format!("{URL}/auth/callback"), "the one address the provider knows");
        assert_eq!(asked["code_challenge_method"], "S256");
        assert!(asked["scope"].split(' ').any(|scope| scope == "openid"), "{}", asked["scope"]);
        let mut inner = self.inner.lock().unwrap();
        inner.made += 1;
        let code = format!("code-{}", inner.made);
        let grant = Grant { challenge: asked["code_challenge"].clone(), nonce: asked["nonce"].clone(), claims };
        inner.grants.insert(code.clone(), grant);
        code
    }
}

fn ed_key(seed: u8) -> EncodingKey {
    // PKCS#8 around an Ed25519 seed.
    let mut der = vec![0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];
    der.extend([seed; 32]);
    EncodingKey::from_ed_der(&der)
}

fn unbase64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let digits = text.bytes().filter_map(|byte| ALPHABET.iter().position(|letter| *letter == byte));
    let (mut out, mut bits, mut held) = (Vec::new(), 0u32, 0);
    for digit in digits {
        bits = bits << 6 | digit as u32;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    out
}

async fn discovery(State(idp): State<Idp>) -> Json<Value> {
    let (issuer, quirk) = (&idp.issuer, idp.inner.lock().unwrap().quirk);
    let methods =
        if quirk == Quirk::SecretInForm { json!(["client_secret_post"]) } else { json!(["client_secret_basic"]) };
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/keys"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "token_endpoint_auth_methods_supported": methods,
        "id_token_signing_alg_values_supported": ["RS256", "EdDSA"],
    }))
}

async fn keys(State(idp): State<Idp>) -> Json<JwkSet> {
    let inner = idp.inner.lock().unwrap();
    let listed = |(name, algorithm, key): &(String, Algorithm, EncodingKey)| {
        let mut jwk = Jwk::from_encoding_key(key, *algorithm).unwrap();
        jwk.common.key_id = Some(name.clone());
        jwk
    };
    Json(JwkSet { keys: inner.keys.iter().map(listed).collect() })
}

fn refused(error: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error, "error_description": "The provider refused." })))
}

async fn token(
    State(idp): State<Idp>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut inner = idp.inner.lock().unwrap();
    let field = |name: &str| form.get(name).map(String::as_str).unwrap_or_default();
    // The client says who it is in the header or in the form, as the provider told it to.
    let header = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let escaped = |text: &str| letters_api::paths::segment(text);
    let basic = format!("Basic {}", base64(format!("{}:{}", escaped(CLIENT), escaped(SECRET)).as_bytes()));
    let known = match inner.quirk {
        Quirk::SecretInForm => header.is_empty() && field("client_id") == CLIENT && field("client_secret") == SECRET,
        _ => header == basic && field("client_secret").is_empty(),
    };
    if !known {
        return Err(refused("invalid_client"));
    }
    // A code is traded once, by whoever holds the secret behind its challenge.
    let Some(grant) = inner.grants.remove(field("code")) else { return Err(refused("invalid_grant")) };
    let shown = base64url(&Sha256::digest(field("code_verifier").as_bytes()));
    let sound = field("grant_type") == "authorization_code"
        && field("redirect_uri") == format!("{URL}/auth/callback")
        && shown == grant.challenge;
    if !sound {
        return Err(refused("invalid_grant"));
    }

    let quirk = inner.quirk;
    let access = format!("access-{}", inner.made);
    let mut claims = grant.claims.as_object().unwrap().clone();
    if quirk == Quirk::ClaimsOnRequest {
        inner.told.insert(access.clone(), Value::Object(claims.clone()));
        claims.retain(|claim, _| claim == "sub");
    }
    claims.insert(
        "iss".into(),
        json!(if quirk == Quirk::OtherIssuer { "https://other.example.test" } else { &inner.issuer }),
    );
    claims.insert("aud".into(), json!(if quirk == Quirk::OtherAudience { "another-client" } else { CLIENT }));
    claims.insert("iat".into(), json!(START));
    claims.insert("exp".into(), json!(if quirk == Quirk::RunOut { START - 3600 } else { START + 300 }));
    claims.insert("nonce".into(), json!(if quirk == Quirk::OtherNonce { "another" } else { &grant.nonce }));
    if quirk == Quirk::OtherParty {
        claims.insert("aud".into(), json!([CLIENT, "another-client"]));
        claims.insert("azp".into(), json!("another-client"));
    }
    let (name, algorithm, key) = inner.keys.last().unwrap();
    let id_token = match quirk {
        Quirk::UnlistedKey => {
            let mut header = Header::new(Algorithm::EdDSA);
            header.kid = Some("ed-unlisted".into());
            encode(&header, &claims, &ed_key(0xee))
        }
        Quirk::SignedWithSecret => {
            let mut header = Header::new(Algorithm::HS256);
            header.kid = Some(name.clone());
            encode(&header, &claims, &EncodingKey::from_secret(SECRET.as_bytes()).unwrap())
        }
        _ => {
            let mut header = Header::new(*algorithm);
            header.kid = Some(name.clone());
            encode(&header, &claims, key)
        }
    };
    Ok(Json(
        json!({ "token_type": "Bearer", "access_token": access, "expires_in": 300, "id_token": id_token.unwrap() }),
    ))
}

async fn userinfo(State(idp): State<Idp>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let header = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let access = header.strip_prefix("Bearer ").ok_or(StatusCode::UNAUTHORIZED)?;
    idp.inner.lock().unwrap().told.get(access).cloned().map(Json).ok_or(StatusCode::UNAUTHORIZED)
}
