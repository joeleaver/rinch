//! Where a link really goes, and whether its text says otherwise.
//!
//! [`check`] reads a link's `href` with the `url` crate (as a browser
//! parses it) and its text, and says what a reader should know before
//! following it ([`Warning`]). [`HtmlView`](crate::HtmlView) shows the
//! answer as the link's tooltip through [`Tooltips`]: by default
//! [`default_tooltip`], in English; an app may give its own words, or none.
//!
//! The site two hosts belong to is compared by their registrable domain, from
//! the Public Suffix List (the `psl` crate): `shop.example.co.uk` is
//! `example.co.uk`, and `evil.github.io` and `good.github.io` are two sites.

use std::rc::Rc;

use url::{Host, Url};

/// What [`check`] found to warn about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The address carries a user name or password before its host
    /// (`https://paypal.com@evil.example/`): what a reader takes for the
    /// site is not where it goes.
    HiddenHost,
    /// The host's name mixes alphabets that look alike (`pаypal.com`, with
    /// a Cyrillic `а`), or is spelled wholly in letters that pass for Latin
    /// ones (`раураl`); `opens` shows it in punycode. An international name in
    /// one script of its own (`münchen.de`, `例え.jp`) is not one.
    LookAlike,
    /// The link's text names a site other than the one it opens.
    Mismatch {
        /// The site the text names.
        named: String,
    },
}

/// What [`check`] says about one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCheck {
    /// The `href` as written.
    pub href: String,
    /// The host it opens, in ASCII (punycode for an international name);
    /// `None` for an address with no host (`mailto:`, a relative link).
    pub opens: Option<String>,
    /// What the reader should know, if anything.
    pub warning: Option<Warning>,
}

/// An app's own tooltip for a link, or `None` for none.
pub type TooltipFn = Rc<dyn Fn(&LinkCheck) -> Option<String>>;

/// How [`HtmlView`](crate::HtmlView) titles its links.
#[derive(Clone, Default)]
pub enum Tooltips {
    /// The address, and a warning in English ([`default_tooltip`]).
    #[default]
    Default,
    /// No tooltip.
    Off,
    /// The app's own: the tooltip for each link, or `None` for none.
    Custom(TooltipFn),
}

impl std::fmt::Debug for Tooltips {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("Default"),
            Self::Off => f.write_str("Off"),
            Self::Custom(_) => f.write_str("Custom(<fn>)"),
        }
    }
}

impl Tooltips {
    /// The tooltip for a link with `href` and `text`.
    pub fn title(&self, href: &str, text: &str) -> Option<String> {
        match self {
            Self::Default => Some(default_tooltip(&check(href, text))),
            Self::Off => None,
            Self::Custom(f) => f(&check(href, text)),
        }
    }
}

/// The address, and the warning in English.
pub fn default_tooltip(link: &LinkCheck) -> String {
    let opens = link.opens.as_deref().unwrap_or_default();
    match &link.warning {
        None => link.href.clone(),
        Some(Warning::HiddenHost) => format!(
            "{}\nCareful: this address hides where it goes. It opens {opens}.",
            link.href
        ),
        Some(Warning::LookAlike) => format!(
            "{}\nCareful: this site's name uses look-alike letters. It is {opens}.",
            link.href
        ),
        Some(Warning::Mismatch { named }) => format!(
            "{}\nCareful: the text says {named}, but this opens {opens}.",
            link.href
        ),
    }
}

/// Top-level domains a bare word in a link's text is read as a site under
/// (`paypal.com`): common ones only, so a file name (`invoice.pdf`,
/// `notes.md`) is not taken for one. A word with a scheme or `www.` counts
/// whatever its domain.
const TEXT_TLDS: &[&str] = &[
    "com", "net", "org", "edu", "gov", "mil", "int", "io", "co", "app", "dev", "ai", "info", "biz",
    "me", "us", "uk", "ca", "au", "nz", "de", "fr", "es", "it", "nl", "be", "ch", "at", "se", "no",
    "dk", "fi", "ie", "jp", "cn", "in", "br", "mx", "ru", "kr", "tv", "xyz", "online", "site",
    "shop", "store", "bank",
];

/// What a reader should know about a link with `href` and `text`.
pub fn check(href: &str, text: &str) -> LinkCheck {
    let parsed = parse(href);
    let opens = parsed
        .as_ref()
        .and_then(|u| u.host())
        .map(|h| host_text(&h));
    let warning = parsed.as_ref().and_then(|url| {
        let opens = opens.as_deref()?;
        if !url.username().is_empty() || url.password().is_some() {
            return Some(Warning::HiddenHost);
        }
        if let Some(named) = text.split_whitespace().find_map(named_host)
            && site(&named) != site(opens)
        {
            return Some(Warning::Mismatch { named });
        }
        looks_alike(opens).then_some(Warning::LookAlike)
    });
    LinkCheck {
        href: href.to_owned(),
        opens,
        warning,
    }
}

/// `href` as a browser reads it, a scheme-relative one (`//host/`) as https.
fn parse(href: &str) -> Option<Url> {
    let href = href.trim();
    match href.strip_prefix("//") {
        Some(rest) => Url::parse(&format!("https://{rest}")).ok(),
        None => Url::parse(href).ok(),
    }
}

/// A host as text, without a leading `www.` or a trailing dot.
fn host_text(host: &Host<&str>) -> String {
    let s = host.to_string();
    let s = s.trim_end_matches('.');
    s.strip_prefix("www.").unwrap_or(s).to_owned()
}

/// Letters of other alphabets drawn like Latin ones.
const LATIN_LOOKING: &str = "аеорсухіјѕԁԛԝҽһӏոօрսνοαιкмнтвʏ";

/// Whether a host (ASCII, punycode for an international name) is written to
/// pass for another: see [`Warning::LookAlike`].
fn looks_alike(host: &str) -> bool {
    if !host.split('.').any(|l| l.starts_with("xn--")) {
        return false;
    }
    let (unicode, _) = idna::domain_to_unicode(host);
    // A name wholly in Cyrillic (or Greek) is how a country that writes in
    // it names things (`москва.ru`); there only a mix of alphabets counts.
    let tld = unicode.rsplit('.').next().unwrap_or_default();
    let own_script_country = matches!(
        tld,
        "ru" | "su"
            | "ua"
            | "by"
            | "bg"
            | "kz"
            | "kg"
            | "mk"
            | "rs"
            | "uz"
            | "tj"
            | "mn"
            | "gr"
            | "cy"
            | "рф"
            | "бел"
            | "укр"
            | "срб"
            | "мкд"
            | "бг"
            | "қаз"
            | "мон"
            | "рус"
            | "ελ"
    );
    unicode.split('.').any(|label| {
        let script = |c: char| match c as u32 {
            0x41..=0x5a | 0x61..=0x7a | 0xc0..=0x24f => Some(0),
            0x370..=0x3ff => Some(1),
            0x400..=0x4ff => Some(2),
            _ => None,
        };
        let scripts: std::collections::HashSet<u8> = label.chars().filter_map(script).collect();
        let mixed = scripts.len() > 1;
        let all_latin_looking = !scripts.contains(&0)
            && label.chars().any(|c| script(c).is_some())
            && label
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| LATIN_LOOKING.contains(c));
        mixed || (all_latin_looking && !own_script_country)
    })
}

/// The site a word in a link's text names, if it names one.
fn named_host(word: &str) -> Option<String> {
    let word = word.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')' | '<' | '>' | '"' | '\'' | ',' | ';' | '!' | '?'
        ) || c == '.'
    });
    let has_scheme = word.contains("://");
    let www = word.to_ascii_lowercase().starts_with("www.");
    let url = if has_scheme {
        Url::parse(word).ok()?
    } else {
        Url::parse(&format!("https://{word}")).ok()?
    };
    let Some(Host::Domain(domain)) = url.host() else {
        return None;
    };
    let tld = domain.rsplit('.').next()?;
    if !domain.contains('.') || (!has_scheme && !www && !TEXT_TLDS.contains(&tld)) {
        return None;
    }
    Some(host_text(&Host::Domain(domain)))
}

/// A host's registrable domain (see the module docs), or the host itself
/// when it has none (an address, a bare suffix).
fn site(host: &str) -> String {
    let host = host.trim_end_matches('.');
    psl::domain_str(host).unwrap_or(host).to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Warning, check};

    fn warning(href: &str, text: &str) -> Option<Warning> {
        check(href, text).warning
    }

    #[test]
    fn plain_links_and_same_site_links_carry_no_warning() {
        assert_eq!(
            warning("https://shop.example/sale?x=1", "See the sale"),
            None
        );
        assert_eq!(warning("https://mail.google.com/x", "www.google.com"), None);
        assert_eq!(warning("mailto:ada@example.org", "Mail Ada"), None);
        assert_eq!(
            warning("https://files.example.com/i.pdf", "invoice.pdf"),
            None
        );
        assert_eq!(warning("https://shop.bbc.co.uk/", "bbc.co.uk"), None);
        // The second review: none of these is a trick.
        assert_eq!(warning("https://münchen.de/", "münchen.de"), None);
        assert_eq!(warning("https://bücher.de/", "Bücher"), None);
        assert_eq!(warning("https://例え.jp/", "例え.jp"), None);
        assert_eq!(warning("https://paypal.com./", "paypal.com"), None);
        assert_eq!(warning("https://москва.ru/", "Москва"), None);
        assert_eq!(
            check("https://xn--80ak6aa92e.com/", "").opens.as_deref(),
            Some("xn--80ak6aa92e.com")
        );
    }

    #[test]
    fn a_link_that_hides_or_disguises_where_it_goes_is_flagged() {
        // The first review of #2213: each of these was silent.
        for href in [
            "https://paypal.com:443@evil.example/",
            "https://paypal.com@evil.example/",
            r"https://evil.example\@paypal.com/",
        ] {
            let c = check(href, "https://paypal.com/");
            assert!(c.warning.is_some(), "{href}: {c:?}");
            assert_ne!(c.opens.as_deref(), Some("paypal.com"), "{href}: {c:?}");
        }
        assert_eq!(
            warning("https://pаypal.com/", "Log in"),
            Some(Warning::LookAlike)
        );
        assert!(matches!(
            warning("https://evil.co.uk/", "barclays.co.uk"),
            Some(Warning::Mismatch { .. })
        ));
        // Shared hosts are many sites (the Public Suffix List).
        for (href, text) in [
            ("https://evil.github.io/", "good.github.io"),
            ("https://evil.blogspot.com/", "good.blogspot.com"),
            ("https://evil.s3.amazonaws.com/", "good.s3.amazonaws.com"),
            // `*.kawasaki.jp` is a suffix; `city.kawasaki.jp` is its listed
            // exception, one site, so its subdomains compare as one.
            ("https://evil.kawasaki.jp/", "good.kawasaki.jp"),
        ] {
            assert!(
                matches!(warning(href, text), Some(Warning::Mismatch { .. })),
                "{href}"
            );
        }
        assert_eq!(
            warning("https://раураl.com/", "Log in"),
            Some(Warning::LookAlike)
        );
        assert!(matches!(
            warning("//evil.example/", "paypal.com"),
            Some(Warning::Mismatch { .. })
        ));
        assert!(matches!(
            warning("https://paypa1-login.example/", "https://www.paypal.com/signin"),
            Some(Warning::Mismatch { named }) if named == "paypal.com"
        ));
    }
}
