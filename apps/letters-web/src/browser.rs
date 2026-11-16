//! What the browser lends the editor: a clock, identifiers, the clipboard for as long as a clipboard event
//! lasts, somewhere to keep the person's own words, default style and settings, the files beside the wasm, and
//! its decoders for pictures.

use std::collections::HashMap;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use letters_editor::{Clip, Picture, Platform, block_id};
use letters_model::Style;
use letters_render::image::Image;
use letters_ui::{Appearance, Settings};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, Crypto, HtmlCanvasElement, ImageBitmap, Performance, RequestInit,
    Response, Storage, Url, Window,
};

/// Fetches the files at `urls`, all at once. A page that preloads them has them here by the time the wasm
/// is running.
pub async fn files(window: &Window, urls: &[String]) -> Result<Vec<Vec<u8>>, JsValue> {
    let requests: Vec<Promise> = urls.iter().map(|url| window.fetch_with_str(url)).collect();
    let mut files = Vec::new();
    for (request, url) in requests.into_iter().zip(urls) {
        let response: Response = JsFuture::from(request).await?.dyn_into()?;
        if !response.ok() {
            return Err(format!("{url} could not be fetched: {}", response.status()).into());
        }
        let bytes = JsFuture::from(response.array_buffer()?).await?;
        files.push(Uint8Array::new(&bytes).to_vec());
    }
    Ok(files)
}

/// Asks the server behind the page for what is at `path`, an address of `letters_api::paths`, with a
/// `body` if it takes one: its media type and its bytes. The browser sends the session along by itself.
/// The request is on its way when this returns, so several asked one after another travel together.
///
/// `Err` is the server's refusal, or one made up here for an answer that never came, with a sentence the
/// person can be shown.
pub fn ask(
    window: &Window,
    method: &str,
    path: &str,
    body: Option<(&str, &[u8])>,
) -> impl Future<Output = Result<Vec<u8>, letters_api::Error>> + 'static + use<> {
    let answered = ask_and_wait(window, method, path, body);
    async move { answered.await.map_err(|(refusal, _)| refusal) }
}

/// As [`ask`], with how many seconds the server said to wait before the same is asked again, where a
/// refusal says: its `Retry-After`.
pub fn ask_and_wait(
    window: &Window,
    method: &str,
    path: &str,
    body: Option<(&str, &[u8])>,
) -> impl Future<Output = Result<Vec<u8>, (letters_api::Error, Option<u32>)>> + 'static + use<> {
    let request = RequestInit::new();
    request.set_method(method);
    if let Some((kind, bytes)) = body {
        let headers = Object::new();
        // A plain object always takes a key.
        let _ = Reflect::set(&headers, &"content-type".into(), &kind.into());
        request.set_headers(&headers);
        request.set_body(&Uint8Array::from(bytes));
    }
    let sent = window.fetch_with_str_and_init(path, &request);
    async move {
        let lost =
            |_| (letters_api::Error::new(letters_api::Code::Unavailable, "The server could not be reached."), None);
        let response: Response = JsFuture::from(sent).await.map_err(lost)?.dyn_into().map_err(lost)?;
        let bytes = JsFuture::from(response.array_buffer().map_err(lost)?).await.map_err(lost)?;
        let bytes = Uint8Array::new(&bytes).to_vec();
        if response.ok() {
            return Ok(bytes);
        }
        let wait = response.headers().get("retry-after").ok().flatten().and_then(|wait| wait.trim().parse().ok());
        // What stands between the page and the server may refuse in words of its own.
        let unread = || {
            let message = format!("The server answered {} and did not say why.", response.status());
            letters_api::Error::new(letters_api::Code::Unknown, message)
        };
        Err((serde_json::from_slice(&bytes).unwrap_or_else(|_| unread()), wait))
    }
}

/// The longest side a picture is drawn from, in pixels. A photo straight from a camera is several times
/// what any screen shows of it, and its pixels would be held twice over while they are handed across.
const LONGEST: f64 = 4096.0;

/// The media type of a picture in one of the formats a document keeps, told by its first bytes.
pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// Has the browser decode a picture: upright, as its EXIF orientation says.
async fn bitmap(window: &Window, picture: &Blob) -> Option<ImageBitmap> {
    let decoding = window.create_image_bitmap_with_blob(picture).ok()?;
    JsFuture::from(decoding).await.ok()?.dyn_into().ok()
}

/// A picture from its file, as the person pasted, dropped or chose it. `None` for what is no picture
/// Letters can show.
pub async fn picture(window: &Window, file: &Blob) -> Option<Picture> {
    let bytes = Uint8Array::new(&JsFuture::from(file.array_buffer()).await.ok()?).to_vec();
    let mime = mime(&bytes)?.to_owned();
    // The size it is shown at is its upright one, which only decoding it tells.
    let decoded = bitmap(window, file).await?;
    let (width, height) = (decoded.width(), decoded.height());
    decoded.close();
    Some(Picture { bytes, mime, width, height })
}

/// A picture from the bytes of its file, as a server fetched them. `None` for what is no picture Letters
/// can show.
pub async fn picture_of(window: &Window, bytes: &[u8]) -> Option<Picture> {
    let kind = BlobPropertyBag::new();
    kind.set_type(mime(bytes)?);
    let file = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), &kind).ok()?;
    picture(window, &file).await
}

/// The pixels of a picture the document keeps, decoded by the browser, which has the decoders already and
/// runs them off this thread.
pub async fn decode(window: &Window, bytes: &[u8]) -> Option<Image> {
    let kind = BlobPropertyBag::new();
    kind.set_type(mime(bytes)?);
    let file = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), &kind).ok()?;
    let decoded = bitmap(window, &file).await?;
    let (width, height) = (f64::from(decoded.width()), f64::from(decoded.height()));
    let scale = (LONGEST / width.max(height)).min(1.0);
    let (width, height) = ((width * scale).round().max(1.0), (height * scale).round().max(1.0));
    // A canvas is the one place a page can read decoded pixels from.
    let canvas: HtmlCanvasElement = window.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width as u32);
    canvas.set_height(height as u32);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.draw_image_with_image_bitmap_and_dw_and_dh(&decoded, 0.0, 0.0, width, height).ok()?;
    decoded.close();
    let pixels = context.get_image_data(0.0, 0.0, width, height).ok()?;
    Image::from_rgba8(pixels.width(), pixels.height(), pixels.data().0).ok()
}

/// Where this browser keeps the person's own words, a line each, as their dictionary file does elsewhere.
const WORDS: &str = "letters.words";

/// Where this browser keeps those of the person's words their account is yet to be told, a line each. A
/// browser that has none has asked no account, and no word of it was told.
const UNSENT: &str = "letters.words.unsent";

/// Where this browser keeps what the person last saw of a server's document, under the document's id after
/// this: the revision it read at, for the note of what others wrote since.
const SEEN: &str = "letters.seen.";

/// Where this browser keeps the person's default style, as a `style.json`, until an account keeps it.
const STYLE: &str = "letters.style";

/// Where this browser keeps what the person set for themselves, a key to each setting, until an account
/// keeps them. A setting that is not set has no key.
const THEME: &str = "letters.theme";
const ACCENT: &str = "letters.accent";
const NAME: &str = "letters.name";
const LANGUAGE: &str = "letters.language";

pub struct Browser {
    performance: Performance,
    crypto: Crypto,
    /// `None` where the person has turned the page's storage off.
    storage: Option<Storage>,
    /// What a `paste` event brought, for the editor to read while it handles that event.
    pub pasted: Option<Clip>,
    /// What the editor copied, for the `copy` or `cut` event that asked for it to carry away.
    pub copied: Option<Clip>,
    /// The pictures put into the document here, by the names it has for them. They are kept until there
    /// is a server to keep them.
    pub assets: HashMap<String, Vec<u8>>,
    /// The names of those the editor put in since the shell last looked, for a server to be handed.
    pub stored: Vec<String>,
    /// The words the person added to their own since the shell last looked, for a server to be told.
    pub learned: Vec<String>,
}

/// The page's async clipboard. Read by name: the binding to it is not in every release of `web-sys`.
fn clipboard(window: &Window) -> Result<JsValue, JsValue> {
    let clipboard = Reflect::get(&window.navigator(), &"clipboard".into())?;
    if clipboard.is_undefined() {
        return Err("this page may not reach the clipboard".into());
    }
    Ok(clipboard)
}

/// Calls a method of the async clipboard, which answers later.
fn call(clipboard: &JsValue, method: &str, argument: Option<&JsValue>) -> Result<Promise, JsValue> {
    let method: js_sys::Function = Reflect::get(clipboard, &method.into())?.dyn_into()?;
    let answer = match argument {
        Some(argument) => method.call1(clipboard, argument)?,
        None => method.call0(clipboard)?,
    };
    answer.dyn_into()
}

fn blob(bytes: &[u8], kind: &str) -> Result<JsValue, JsValue> {
    let bag = BlobPropertyBag::new();
    bag.set_type(kind);
    Ok(Blob::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), &bag)?.into())
}

/// Puts a clip on the clipboard outside a clipboard event, as a press on a copy button asks: a picture
/// alone as that picture, and words as text and, where they have it, HTML. The browser allows it only in
/// the wake of a press or a key, and not at all on a page served over plain HTTP.
pub fn copy(window: &Window, clip: &Clip) -> Result<(), JsValue> {
    let clipboard = clipboard(window)?;
    let picture = clip.pictures.first().filter(|_| clip.text.is_empty() && clip.html.is_none());
    let kinds = Object::new();
    if let Some(picture) = picture {
        Reflect::set(&kinds, &picture.mime.as_str().into(), &blob(&picture.bytes, &picture.mime)?)?;
    } else if let Some(html) = &clip.html {
        Reflect::set(&kinds, &"text/plain".into(), &blob(clip.text.as_bytes(), "text/plain")?)?;
        Reflect::set(&kinds, &"text/html".into(), &blob(html.as_bytes(), "text/html")?)?;
    }
    let item = Reflect::get(window, &"ClipboardItem".into())?;
    let written = match item.dyn_ref::<js_sys::Function>() {
        Some(item) if picture.is_some() || clip.html.is_some() => {
            let item = Reflect::construct(item, &Array::of1(&kinds))?;
            call(&clipboard, "write", Some(&Array::of1(&item)))?
        }
        // A browser without clipboard items has no way to take a picture.
        _ if picture.is_some() => return Err("this browser cannot copy a picture".into()),
        _ => call(&clipboard, "writeText", Some(&clip.text.as_str().into()))?,
    };
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(refused) = JsFuture::from(written).await {
            web_sys::console::error_1(&refused);
        }
    });
    Ok(())
}

/// What a blob of text holds.
async fn words(held: &Blob) -> Option<String> {
    JsFuture::from(held.text()).await.ok()?.as_string()
}

/// Reads the clipboard outside a paste event, as a menu's Paste asks. The browser asks the person first,
/// or shows a button of its own to press, and gives nothing when they decline. A browser that hands over
/// only text is asked for that.
pub async fn paste(window: &Window) -> Option<Clip> {
    let clipboard = clipboard(window).ok()?;
    let Ok(items) = call(&clipboard, "read", None) else {
        let text = JsFuture::from(call(&clipboard, "readText", None).ok()?).await.ok()?.as_string()?;
        return Some(Clip { text, ..Clip::default() }).filter(|clip| !clip.text.is_empty());
    };
    let items: Array = JsFuture::from(items).await.ok()?.dyn_into().ok()?;
    let mut clip = Clip::default();
    for item in items.iter() {
        let kinds: Array = Reflect::get(&item, &"types".into()).ok()?.dyn_into().ok()?;
        for kind in kinds.iter().filter_map(|kind| kind.as_string()) {
            let Ok(held) = call(&item, "getType", Some(&kind.as_str().into())) else { continue };
            let Some(held) = JsFuture::from(held).await.ok().and_then(|held| held.dyn_into::<Blob>().ok()) else {
                continue;
            };
            match kind.as_str() {
                "text/plain" => clip.text = words(&held).await.unwrap_or_default(),
                "text/html" => clip.html = words(&held).await.filter(|html| !html.is_empty()),
                kind if kind.starts_with("image/") => clip.pictures.extend(picture(window, &held).await),
                _ => {}
            }
        }
    }
    // Pictures beside words are a rendering of the words.
    if !clip.text.is_empty() {
        clip.pictures.clear();
    }
    Some(clip).filter(|clip| !clip.text.is_empty() || clip.html.is_some() || !clip.pictures.is_empty())
}

impl Browser {
    /// What makes a document that one and no other: a new id, and now as RFC 3339 to the second, which
    /// is how every writer of the format dates a document.
    pub fn identity(&self) -> [String; 2] {
        let now = String::from(js_sys::Date::new_0().to_iso_string());
        [self.crypto.random_uuid(), format!("{}Z", now.get(..19).unwrap_or(&now))]
    }

    pub fn new(window: &Window) -> Result<Self, JsValue> {
        let performance = window.performance().ok_or("this browser has no clock")?;
        let storage = window.local_storage().ok().flatten();
        Ok(Self {
            performance,
            crypto: window.crypto()?,
            storage,
            pasted: None,
            copied: None,
            assets: HashMap::new(),
            stored: Vec::new(),
            learned: Vec::new(),
        })
    }

    /// Whether the browser had `url` from an earlier visit when the page asked for it, as its preload did.
    /// What came from the cache crossed no network, or only as much of one as asking whether it had changed.
    pub fn kept(&self, window: &Window, url: &str) -> bool {
        let page = window.location().href().unwrap_or_default();
        let Ok(resolved) = Url::new_with_base(url, &page) else { return false };
        let Some(entry) = self.performance.get_entries_by_name(&resolved.href()).iter().last() else { return false };
        let size = |name: &str| Reflect::get(&entry, &name.into()).ok().and_then(|size| size.as_f64());
        // A browser that does not say is taken to have fetched it.
        matches!((size("transferSize"), size("encodedBodySize")), (Some(sent), Some(held)) if sent < held)
    }

    /// The words the person has added to their own dictionary in this browser.
    pub fn user_words(&self) -> Vec<String> {
        let kept = self.storage.as_ref().and_then(|storage| storage.get_item(WORDS).ok().flatten());
        kept.unwrap_or_default().lines().map(str::to_owned).collect()
    }

    /// Keeps `words` as the person's own, in place of what this browser kept.
    pub fn keep_user_words(&self, words: &[String]) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(WORDS, &words.join("\n"));
        }
    }

    /// The words of this browser that the person's account is yet to be told.
    pub fn unsent_words(&self) -> Vec<String> {
        let kept = self.storage.as_ref().and_then(|storage| storage.get_item(UNSENT).ok().flatten());
        kept.map_or_else(|| self.user_words(), |kept| kept.lines().map(str::to_owned).collect())
    }

    /// Keeps `words` as those the person's account is yet to be told.
    pub fn keep_unsent_words(&self, words: &[String]) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(UNSENT, &words.join("\n"));
        }
    }

    /// Takes a word out of the person's own, as they asked.
    pub fn forget_word(&self, word: &str) {
        let without = |words: Vec<String>| words.into_iter().filter(|known| known != word).collect::<Vec<_>>();
        self.keep_user_words(&without(self.user_words()));
        if self.told_an_account() {
            self.keep_unsent_words(&without(self.unsent_words()));
        }
    }

    /// Whether an account was ever told this browser's words.
    fn told_an_account(&self) -> bool {
        self.storage.as_ref().is_some_and(|storage| matches!(storage.get_item(UNSENT), Ok(Some(_))))
    }

    /// The revision the document of this id read at when the person last had it open in this browser.
    pub fn last_seen(&self, id: &str) -> Option<String> {
        self.storage.as_ref()?.get_item(&format!("{SEEN}{id}")).ok().flatten()
    }

    /// Keeps `revision` as what the person last saw of the document of this id.
    pub fn keep_last_seen(&self, id: &str, revision: &str) {
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(&format!("{SEEN}{id}"), revision);
        }
    }

    /// The `style.json` the person kept as their default in this browser, if they kept one.
    pub fn kept_default_style(&self) -> Option<Value> {
        let kept = self.storage.as_ref()?.get_item(STYLE).ok().flatten()?;
        serde_json::from_str(&kept).ok()
    }

    /// The style the person's new documents start with: the one they kept in this browser, else the
    /// built-in one. `None` where the browser keeps nothing, so there is no default to change.
    pub fn default_style(&self) -> Option<Style> {
        let kept = self.storage.as_ref()?.get_item(STYLE).ok().flatten();
        Some(kept.and_then(|json| Style::from_json(&json).ok()).unwrap_or_default())
    }

    /// Keeps `json`, a `style.json`, as the person's default style. `false` when it could not be kept.
    pub fn keep_default_style(&self, json: &str) -> bool {
        self.storage.as_ref().is_some_and(|storage| storage.set_item(STYLE, json).is_ok())
    }

    /// What the person set for themselves in this browser. A key that holds what makes no sense for it is
    /// as if it had never been set.
    pub fn settings(&self) -> Settings {
        let kept = |key| {
            let kept = self.storage.as_ref()?.get_item(key).ok().flatten()?;
            Some(kept.trim().to_owned()).filter(|kept| !kept.is_empty())
        };
        Settings {
            appearance: match kept(THEME).as_deref() {
                Some("light") => Appearance::Light,
                Some("dark") => Appearance::Dark,
                _ => Appearance::System,
            },
            accent: kept(ACCENT).and_then(|accent| colour(&accent)),
            name: kept(NAME),
            language: kept(LANGUAGE),
        }
    }

    /// Keeps `settings` for the next page this browser loads, and for its other tabs. Storage that is full
    /// or turned off keeps nothing, and the settings still hold until the page closes.
    pub fn keep_settings(&self, settings: &Settings) {
        let Some(storage) = &self.storage else { return };
        let theme = match settings.appearance {
            Appearance::System => None,
            Appearance::Light => Some("light".to_owned()),
            Appearance::Dark => Some("dark".to_owned()),
        };
        let accent = settings.accent.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
        let keys =
            [(THEME, theme), (ACCENT, accent), (NAME, settings.name.clone()), (LANGUAGE, settings.language.clone())];
        for (key, set) in keys {
            let _ = match set {
                Some(set) => storage.set_item(key, &set),
                None => storage.remove_item(key),
            };
        }
    }

    /// Milliseconds since the page started loading, to a fraction.
    pub fn now(&self) -> f64 {
        self.performance.now()
    }
}

/// Reads `#rrggbb`.
fn colour(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    // A sign would parse as a number, and is no part of a colour.
    if digits.len() != 6 || !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(digits, 16).ok()?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

impl Platform for Browser {
    fn now_ms(&self) -> u64 {
        self.now() as u64
    }

    // Edits are dated by the browser's clock.
    fn wall_time(&self) -> Option<u64> {
        Some((js_sys::Date::now() / 1000.0) as u64)
    }

    // A page may read the clipboard only inside a paste event, so outside one there is nothing to give.
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.pasted.clone()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.copied = Some(clip);
    }

    fn new_id(&mut self) -> String {
        let mut bytes = [0; 10];
        self.crypto.get_random_values_with_u8_array(&mut bytes).expect("the browser has a source of randomness");
        block_id(bytes)
    }

    fn store_asset(&mut self, name: &str, bytes: &[u8]) {
        if !self.assets.contains_key(name) {
            self.assets.insert(name.to_owned(), bytes.to_vec());
            self.stored.push(name.to_owned());
        }
    }

    fn learn_word(&mut self, word: &str) {
        let mut words = self.user_words();
        if words.iter().any(|known| known == word) {
            return;
        }
        words.push(word.to_owned());
        // Storage that is full or turned off keeps nothing, and the word still holds until the page closes.
        self.keep_user_words(&words);
        // Until an account takes it: the page may close first, or show to someone with no account.
        if self.told_an_account() {
            let mut unsent = self.unsent_words();
            unsent.push(word.to_owned());
            self.keep_unsent_words(&unsent);
        }
        self.learned.push(word.to_owned());
    }
}
