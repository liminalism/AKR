//! Identifiers and scalar literals: segments, keys, dates, commits and globs.

use std::fmt;

/// Why an identifier or literal was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentError {
    /// A segment did not match `[a-z][a-z0-9]*(-[a-z0-9]+)*` (D-005).
    BadSegment(String),
    /// A key had fewer than two or more than eight segments (D-005).
    BadKeyLength(usize),
    /// A commit was not exactly 40 lowercase hex digits (D-008).
    BadCommit(String),
    /// A date was not a valid calendar date (D-008).
    BadDate(String),
}

impl fmt::Display for IdentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSegment(s) => write!(
                f,
                "{s:?} is not a valid segment: it must start with a lowercase letter and \
                 hold only lowercase letters, digits and single hyphens"
            ),
            Self::BadKeyLength(n) => write!(f, "a key has 2 to 8 segments, found {n}"),
            Self::BadCommit(s) => write!(f, "{s:?} is not 40 lowercase hex digits"),
            Self::BadDate(s) => write!(f, "{s:?} is not a valid calendar date"),
        }
    }
}

impl std::error::Error for IdentError {}

/// A key segment, enum value, or anchor: `[a-z][a-z0-9]*(-[a-z0-9]+)*` (D-005).
///
/// Segments carry hyphens and never underscores, which is what makes them lexically
/// distinguishable from slot names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Segment(String);

impl Segment {
    /// Parses a segment, rejecting anything outside the D-005 charset.
    ///
    /// # Errors
    /// Returns [`IdentError::BadSegment`] if the text is not a valid segment.
    pub fn new(text: &str) -> Result<Self, IdentError> {
        let bad = || IdentError::BadSegment(text.to_owned());
        let bytes = text.as_bytes();
        if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
            return Err(bad());
        }
        let mut prev_hyphen = false;
        for &b in &bytes[1..] {
            match b {
                b'a'..=b'z' | b'0'..=b'9' => prev_hyphen = false,
                b'-' if !prev_hyphen => prev_hyphen = true,
                _ => return Err(bad()),
            }
        }
        if prev_hyphen {
            return Err(bad());
        }
        Ok(Self(text.to_owned()))
    }

    /// The segment text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A logical record key: two to eight segments joined by `.` (D-005).
///
/// Identity. Never renamed, never reused, never derived from a filename (D-018).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalKey(Vec<Segment>);

impl LogicalKey {
    /// Parses a dotted key.
    ///
    /// # Errors
    /// Returns an error if any segment is malformed or the segment count is outside 2..=8.
    pub fn parse(text: &str) -> Result<Self, IdentError> {
        let segments = text
            .split('.')
            .map(Segment::new)
            .collect::<Result<Vec<_>, _>>()?;
        if !(2..=8).contains(&segments.len()) {
            return Err(IdentError::BadKeyLength(segments.len()));
        }
        Ok(Self(segments))
    }

    /// The first segment, which must be a declared namespace (V-002).
    #[must_use]
    pub fn namespace(&self) -> &Segment {
        &self.0[0]
    }

    /// All segments, in order.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.0
    }
}

impl fmt::Display for LogicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for s in &self.0 {
            if !first {
                f.write_str(".")?;
            }
            first = false;
            f.write_str(s.as_str())?;
        }
        Ok(())
    }
}

/// A git commit: exactly 40 lowercase hex digits (D-008).
///
/// Abbreviations are rejected because they collide as history grows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Commit(String);

impl Commit {
    /// Parses a commit hash, with or without the `git:` prefix.
    ///
    /// # Errors
    /// Returns [`IdentError::BadCommit`] unless the hash is 40 lowercase hex digits.
    pub fn new(text: &str) -> Result<Self, IdentError> {
        let hex = text.strip_prefix("git:").unwrap_or(text);
        if hex.len() == 40
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(hex.to_owned()))
        } else {
            Err(IdentError::BadCommit(text.to_owned()))
        }
    }

    /// The 40 hex digits, without the `git:` prefix.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "git:{}", self.0)
    }
}

/// A calendar date (D-008). No time, no zone; timestamps are UTC-only and separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Year.
    pub year: i32,
    /// Month, 1..=12.
    pub month: u8,
    /// Day, 1..=31 as the month allows.
    pub day: u8,
}

impl Date {
    /// Builds a date, rejecting impossible ones.
    ///
    /// # Errors
    /// Returns [`IdentError::BadDate`] for an out-of-range month or day.
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, IdentError> {
        let bad = || IdentError::BadDate(format!("{year:04}-{month:02}-{day:02}"));
        if !(1..=12).contains(&month) || day == 0 {
            return Err(bad());
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let max = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            _ if leap => 29,
            _ => 28,
        };
        if day > max {
            Err(bad())
        } else {
            Ok(Self { year, month, day })
        }
    }

    /// Parses `YYYY-MM-DD`.
    ///
    /// # Errors
    /// Returns [`IdentError::BadDate`] if the shape or the values are wrong.
    pub fn parse(text: &str) -> Result<Self, IdentError> {
        let bad = || IdentError::BadDate(text.to_owned());
        let (y, rest) = text.split_once('-').ok_or_else(bad)?;
        let (m, d) = rest.split_once('-').ok_or_else(bad)?;
        if y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return Err(bad());
        }
        Self::new(
            y.parse().map_err(|_| bad())?,
            m.parse().map_err(|_| bad())?,
            d.parse().map_err(|_| bad())?,
        )
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A repo-root-relative path glob over the D-008 subset: `*`, `**`, `?`, `[...]`.
///
/// No brace expansion and no negation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Glob(String);

impl Glob {
    /// Wraps a glob string.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self(text.to_owned())
    }

    /// The glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The literal segment prefix: every leading path segment containing no wildcard.
    ///
    /// This is the whole basis of the conservative overlap test of D-010.
    #[must_use]
    pub fn literal_prefix(&self) -> Vec<&str> {
        self.0
            .split('/')
            .take_while(|seg| !seg.contains(['*', '?', '[']))
            .filter(|seg| !seg.is_empty())
            .collect()
    }
}

impl fmt::Display for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
