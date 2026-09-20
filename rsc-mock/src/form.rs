//! Decoding of `application/x-www-form-urlencoded` data, used for both URL queries and request
//! bodies.

use url::form_urlencoded;

/// The decoded name/value pairs of a query or form body, in order of appearance.
///
/// Decoding is lossy (`+` is a space, `%XX` is percent-decoding, invalid UTF-8 becomes U+FFFD),
/// so no input can make it fail.
#[derive(Debug)]
pub(crate) struct Params(Vec<(String, String)>);

/// A parameter was present more than once.
#[derive(Debug)]
pub(crate) struct Repeated;

impl Params {
    pub(crate) fn parse(raw: &[u8]) -> Self {
        Self(
            form_urlencoded::parse(raw)
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect(),
        )
    }

    /// Whether the parameter appears at all, even with an empty value.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.occurrences(name).next().is_some()
    }

    /// The value of the first occurrence, even when empty. Used for `state`, which is echoed
    /// verbatim.
    pub(crate) fn first(&self, name: &str) -> Option<&str> {
        self.occurrences(name).next()
    }

    /// The value of the first occurrence, `None` when absent or empty: RFC 6749 sections 3.1 and
    /// 3.2 say a parameter sent without a value is treated as omitted.
    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        self.first(name).filter(|value| !value.is_empty())
    }

    /// Like [`Params::value`], but a parameter present more than once is an error (RFC 6749
    /// section 3.1: parameters MUST NOT be included more than once).
    pub(crate) fn unique(&self, name: &str) -> Result<Option<&str>, Repeated> {
        let mut values = self.occurrences(name);
        let first = values.next();
        if values.next().is_some() {
            return Err(Repeated);
        }
        Ok(first.filter(|value| !value.is_empty()))
    }

    fn occurrences<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a str> {
        self.0
            .iter()
            .filter(move |(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}
