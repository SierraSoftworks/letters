//! The Google Fonts library: the families a writer can choose beyond the bundled ones and the device's,
//! which every reader can fetch by name. All are under open licences.
//!
//! Nothing here fetches: a shell or a server asks for [`Family::stylesheet`], reads the face's address
//! out of the answer with [`face`], and fetches that. `library.txt` lists the families, and
//! `scripts/font-library.py` writes it.

/// The host that answers [`Family::stylesheet`].
pub const STYLESHEETS: &str = "fonts.googleapis.com";
/// The host that holds the faces a stylesheet names.
pub const FACES: &str = "fonts.gstatic.com";

#[cfg(feature = "library")]
const LISTED: &str = include_str!("../library.txt");

/// A family of the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Family<'a> {
    pub name: &'a str,
    /// The weights it has: `100-900` for all between, else each one.
    weights: &'a str,
    /// Whether it has italics of its own.
    pub italic: bool,
}

/// The list of the families as `library.txt` writes it, for a server to hand to a page that has none.
#[cfg(feature = "library")]
pub fn listing() -> &'static str {
    LISTED
}

/// Every family, by name.
#[cfg(feature = "library")]
pub fn families() -> impl Iterator<Item = Family<'static>> {
    read(LISTED)
}

/// The family of a name, however its letters are cased.
#[cfg(feature = "library")]
pub fn find(name: &str) -> Option<Family<'static>> {
    find_in(LISTED, name)
}

/// Every family a [`listing`] names, in its order.
pub fn read(listing: &str) -> impl Iterator<Item = Family<'_>> {
    listing.lines().filter(|line| !line.starts_with('#')).filter_map(|line| {
        let mut parts = line.split('\t');
        Some(Family { name: parts.next()?, weights: parts.next()?, italic: parts.next()? == "1" })
    })
}

/// The family of a name in a [`listing`], however its letters are cased.
pub fn find_in<'a>(listing: &'a str, name: &str) -> Option<Family<'a>> {
    read(listing).find(|family| family.name.eq_ignore_ascii_case(name.trim()))
}

impl Family<'_> {
    /// The weight it has nearest to the one asked for, in whole hundreds as the library serves them.
    pub fn weight(&self, asked: u16) -> u16 {
        let asked = (asked.clamp(100, 900) + 50) / 100 * 100;
        let number = |weight: &str| weight.parse::<u16>().ok();
        if let Some((least, most)) = self.weights.split_once('-') {
            let (least, most) = (number(least).unwrap_or(400), number(most).unwrap_or(400));
            // An axis may end between two hundreds, and the library serves the hundreds inside it.
            return asked.clamp(least.div_ceil(100) * 100, most / 100 * 100);
        }
        let has = self.weights.split(',').filter_map(number);
        has.min_by_key(|weight| weight.abs_diff(asked)).unwrap_or(400)
    }

    /// Where the stylesheet is that names one face: the weight nearest to the one asked for, in italics
    /// where the family has them. Asked with no `User-Agent`, the answer names a whole TrueType file,
    /// which [`face`] reads out of it.
    pub fn stylesheet(&self, weight: u16, italic: bool) -> String {
        let (name, weight) = (self.name.replace(' ', "+"), self.weight(weight));
        match self.italic {
            true => format!("https://{STYLESHEETS}/css2?family={name}:ital,wght@{},{weight}", u8::from(italic)),
            false => format!("https://{STYLESHEETS}/css2?family={name}:wght@{weight}"),
        }
    }
}

/// The address of the face a stylesheet names, when it is a TrueType file of the library's own host.
pub fn face(stylesheet: &str) -> Option<&str> {
    let from = stylesheet.find("url(")? + 4;
    let address = stylesheet[from..].split(')').next()?.trim_matches(['"', '\'']);
    let path = address.strip_prefix("https://")?.strip_prefix(FACES)?;
    let plain = path.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/-_.".contains(&byte));
    (path.starts_with('/') && plain && !path.contains("..") && path.ends_with(".ttf")).then_some(address)
}

#[cfg(all(test, feature = "library"))]
mod tests {
    use super::*;

    #[test]
    fn the_library_lists_its_families_by_name() {
        assert!(families().count() > 1_500, "the list is whole");
        let names: Vec<&str> = families().map(|family| family.name).collect();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]), "each once, in order");
        assert_eq!(find(" open sans ").map(|family| family.name), Some("Open Sans"));
        assert_eq!(find("Helvetica Neue"), None, "a font of a device is not the library's");
    }

    #[test]
    fn a_face_is_asked_for_at_the_nearest_weight_the_family_has() {
        // The family, the face asked for, and what the stylesheet's address ends in.
        let cases = [
            ("Lora", 400, false, "family=Lora:ital,wght@0,400"),
            ("Lora", 700, true, "family=Lora:ital,wght@1,700"),
            ("Lora", 300, false, "family=Lora:ital,wght@0,400"),
            ("Open Sans", 650, true, "family=Open+Sans:ital,wght@1,700"),
            ("Lobster", 700, true, "family=Lobster:wght@400"),
            ("Noto Sans JP", 950, false, "family=Noto+Sans+JP:wght@900"),
        ];
        for (name, weight, italic, ends) in cases {
            let address = find(name).unwrap_or_else(|| panic!("{name} is listed")).stylesheet(weight, italic);
            assert!(address.starts_with("https://fonts.googleapis.com/css2?"), "{address}");
            assert!(address.ends_with(ends), "{name} {weight} {italic}: {address}");
        }
    }

    #[test]
    fn only_a_truetype_file_of_the_library_is_read_out_of_a_stylesheet() {
        let sheet = |address: &str| {
            format!("@font-face {{\n  font-family: 'Lora';\n  src: url({address}) format('truetype');\n}}")
        };
        // The address a stylesheet names, and whether it is taken.
        let cases = [
            ("https://fonts.gstatic.com/s/lora/v37/0QI6MX1D_JOuGQbT0gvTJPa787weuyJG.ttf", true),
            ("https://fonts.gstatic.com/s/lora/v37/0QIhMX1D_JOuMw_LJftLp_A.woff2", false),
            ("https://fonts.gstatic.com.example.org/s/lora.ttf", false),
            ("https://example.org/lora.ttf", false),
            ("http://fonts.gstatic.com/s/lora.ttf", false),
            ("https://fonts.gstatic.com/s/../lora.ttf", false),
            ("https://fonts.gstatic.com/s/lora.ttf?next=.ttf", false),
        ];
        for (address, taken) in cases {
            assert_eq!(face(&sheet(address)), taken.then_some(address), "{address}");
        }
        assert_eq!(face("/* no such family */"), None);
    }
}
