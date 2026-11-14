//! Folder paths, compared part by part without touching the disk.

/// An absolute path. The shell resolves links and `..` before asking, so a path that still climbs is
/// never read.
#[derive(Clone, Debug)]
pub(crate) struct Path {
    /// Windows paths take either separator and ignore case.
    windows: bool,
    parts: Vec<String>,
}

impl Path {
    /// Reads an absolute path, or one under `~` when the home folder is known.
    pub(crate) fn parse(text: &str, home: Option<&str>) -> Option<Self> {
        let expanded;
        let text = match text.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                expanded = format!("{}{rest}", home?);
                &expanded
            }
            Some(_) => return None,
            None => text,
        };
        // Windows writes a resolved path as `\\?\C:\...`.
        let text = text.strip_prefix(r"\\?\").unwrap_or(text);
        let drive = matches!(text.as_bytes(), [letter, b':', b'/' | b'\\', ..] if letter.is_ascii_alphabetic());
        let windows = drive || text.starts_with(r"\\");
        if !windows && !text.starts_with('/') {
            return None;
        }
        let separators: &[char] = if windows { &['/', '\\'] } else { &['/'] };
        let parts: Vec<String> =
            text.split(separators).filter(|part| !part.is_empty() && *part != ".").map(String::from).collect();
        (!parts.iter().any(|part| part == "..")).then_some(Self { windows, parts })
    }

    /// Whether this is narrow enough to trust: a folder, not a whole disk.
    pub(crate) fn is_folder(&self) -> bool {
        self.parts.len() > usize::from(self.windows)
    }

    /// Whether `other` is this folder or lies anywhere inside it.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        let same = |a: &String, b: &String| if self.windows { a.eq_ignore_ascii_case(b) } else { a == b };
        self.is_folder()
            && self.windows == other.windows
            && self.parts.len() <= other.parts.len()
            && self.parts.iter().zip(&other.parts).all(|(a, b)| same(a, b))
    }

    pub(crate) fn is(&self, other: &Self) -> bool {
        self.contains(other) && other.contains(self)
    }

    /// The path as the trust file stores it: under `~` where it can be, so the file reads the same on
    /// every device that shares it.
    pub(crate) fn written(&self, home: Option<&str>) -> String {
        if let Some(home) = home.and_then(|home| Self::parse(home, None)).filter(|home| home.contains(self)) {
            let rest = &self.parts[home.parts.len()..];
            return std::iter::once("~").chain(rest.iter().map(String::as_str)).collect::<Vec<_>>().join("/");
        }
        match self.windows {
            true if !self.parts.first().is_some_and(|part| part.ends_with(':')) => {
                format!(r"\\{}", self.parts.join(r"\"))
            }
            true => self.parts.join(r"\"),
            false => format!("/{}", self.parts.join("/")),
        }
    }
}
