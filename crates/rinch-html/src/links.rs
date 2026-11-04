//! Where a link really goes, and whether its text says otherwise.
//!
//! [`check`] reads a link's `href` with the `url` crate (as a browser
//! parses it) and its text, and says what a reader should know before
//! following it ([`Warning`]). [`HtmlView`](crate::HtmlView) shows the
//! answer as the link's tooltip through [`Tooltips`]: by default
//! [`default_tooltip`], in English; an app may give its own words, or none.
//!
//! The site two hosts belong to is compared by their registrable domain: the
//! label before the public suffix. There is no full public-suffix list here;
//! [`TWO_LABEL_SUFFIXES`] holds the common two-label ones (`co.uk`,
//! `com.au`, …), and any other suffix is taken as the last label.

use std::rc::Rc;

use url::{Host, Url};

/// What [`check`] found to warn about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The address carries a user name or password before its host
    /// (`https://paypal.com@evil.example/`): what a reader takes for the
    /// site is not where it goes.
    HiddenHost,
    /// The host is spelled with letters outside ASCII, which can look like
    /// another site's (`pаypal.com` with a Cyrillic `а`); `opens` shows it
    /// in punycode.
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

/// Two-label public suffixes: a host under one is compared by the label
/// before them (`shop.example.co.uk` is `example.co.uk`).
pub const TWO_LABEL_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "ltd.uk", "plc.uk", "me.uk", "net.uk", "com.au",
    "net.au", "org.au", "edu.au", "gov.au", "co.nz", "org.nz", "govt.nz", "co.jp", "ne.jp",
    "or.jp", "ac.jp", "co.kr", "or.kr", "co.in", "net.in", "org.in", "gov.in", "com.br", "net.br",
    "org.br", "gov.br", "com.mx", "org.mx", "gob.mx", "co.za", "org.za", "com.cn", "net.cn",
    "org.cn", "gov.cn", "com.tw", "org.tw", "com.hk", "org.hk", "com.sg", "org.sg", "com.ar",
    "com.tr", "co.il", "org.il", "co.id", "or.id", "com.my", "com.ph", "com.pk", "com.ng", "co.ke",
    "com.eg", "com.sa", "com.ua", "co.th", "in.th",
];

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
        if opens.split('.').any(|label| label.starts_with("xn--")) {
            return Some(Warning::LookAlike);
        }
        let named = text.split_whitespace().find_map(named_host)?;
        (site(&named) != site(opens)).then_some(Warning::Mismatch { named })
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

/// A host as text, without a leading `www.`.
fn host_text(host: &Host<&str>) -> String {
    let s = host.to_string();
    s.strip_prefix("www.").map(str::to_owned).unwrap_or(s)
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

/// A host's registrable domain (see the module docs).
fn site(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let take = if labels.len() >= 3
        && TWO_LABEL_SUFFIXES.contains(&labels[labels.len() - 2..].join(".").as_str())
    {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(take)..].join(".")
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
