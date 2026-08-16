//! File-level mechanics of reading a PSS/E raw file.
//!
//! Raw files are line oriented and comma delimited, but with enough quirks that the splitting
//! deserves its own layer: fields may be quoted (single or double), an unquoted `/` starts a
//! trailing comment, sections are delimited by `0 / END OF x DATA, BEGIN y DATA` markers, and
//! v35 sprinkles `@!` column-header comments throughout.

use std::fmt;

/// Decode raw bytes as Latin-1.
///
/// Raw files predate UTF-8 and routinely carry CP-1252 bytes in their free-text comment lines
/// (the sample case has curly quotes at 0x93/0x94). Every byte maps to a char, so this never
/// fails and never mangles the ASCII that all the actual data is written in.
pub fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

#[derive(Debug)]
pub enum PsseError {
    Io(std::io::Error),
    /// The file's `REV` field names a revision we have no field map for.
    UnsupportedRevision(i32),
    /// A record was malformed or a required field was missing.
    Parse {
        line: usize,
        message: String,
    },
    /// The file ended (or a section ended) in the middle of a multi-line record.
    Truncated {
        line: usize,
        what: String,
    },
}

impl fmt::Display for PsseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PsseError::Io(e) => write!(f, "{e}"),
            PsseError::UnsupportedRevision(rev) => {
                write!(f, "unsupported PSS/E revision {rev} (supported: 32-35)")
            }
            PsseError::Parse { line, message } => write!(f, "line {line}: {message}"),
            PsseError::Truncated { line, what } => {
                write!(f, "line {line}: file ended in the middle of {what}")
            }
        }
    }
}

impl std::error::Error for PsseError {}

impl From<std::io::Error> for PsseError {
    fn from(e: std::io::Error) -> Self {
        PsseError::Io(e)
    }
}

/// One comma-delimited field, remembering whether it was quoted.
///
/// Quoting matters for more than whitespace: the switched shunt layout shifted between
/// revisions, and the only stable landmark in the record is its quoted `RMIDNT` field.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub text: String,
    pub quoted: bool,
}

impl Field {
    fn unquoted(text: &str) -> Field {
        Field {
            text: text.trim().to_string(),
            quoted: false,
        }
    }
}

/// A parsed record: its fields, any trailing comment, and the line it came from.
#[derive(Debug, Clone)]
pub struct Record {
    pub fields: Vec<Field>,
    pub comment: Option<String>,
    pub line: usize,
}

impl Record {
    /// True for the `0 /END OF ... DATA` lines that delimit sections.
    ///
    /// A leading zero alone is not enough: transformer impedance lines can open with one. The
    /// distinguishing feature is that nothing else on the line carries data.
    fn is_terminator(&self) -> bool {
        let mut fields = self.fields.iter();
        match fields.next() {
            Some(first) if first.text == "0" && !first.quoted => {
                fields.all(|f| f.text.is_empty() && !f.quoted)
            }
            _ => false,
        }
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn text(&self, idx: usize) -> Result<&str, PsseError> {
        self.fields
            .get(idx)
            .map(|f| f.text.as_str())
            .ok_or_else(|| self.missing(idx))
    }

    /// Field `idx` as text, or `""` if the record stops short of it.
    ///
    /// Raw files may omit trailing fields that would take their default value.
    pub fn opt_text(&self, idx: usize) -> &str {
        self.fields.get(idx).map(|f| f.text.as_str()).unwrap_or("")
    }

    pub fn f64(&self, idx: usize) -> Result<f64, PsseError> {
        let text = self.text(idx)?;
        text.parse::<f64>().map_err(|_| PsseError::Parse {
            line: self.line,
            message: format!("field {idx} ({text:?}) is not a number"),
        })
    }

    pub fn opt_f64(&self, idx: usize, default: f64) -> f64 {
        self.f64(idx).unwrap_or(default)
    }

    /// Field `idx` as an integer, tolerating values written with a decimal point.
    pub fn i32(&self, idx: usize) -> Result<i32, PsseError> {
        let text = self.text(idx)?;
        if let Ok(v) = text.parse::<i32>() {
            return Ok(v);
        }
        text.parse::<f64>()
            .map(|v| v as i32)
            .map_err(|_| PsseError::Parse {
                line: self.line,
                message: format!("field {idx} ({text:?}) is not an integer"),
            })
    }

    pub fn opt_i32(&self, idx: usize, default: i32) -> i32 {
        self.i32(idx).unwrap_or(default)
    }

    /// Index of the last quoted field, used to anchor layouts that shifted between revisions.
    pub fn last_quoted(&self) -> Option<usize> {
        self.fields.iter().rposition(|f| f.quoted)
    }

    fn missing(&self, idx: usize) -> PsseError {
        PsseError::Parse {
            line: self.line,
            message: format!("expected at least {} fields, found {}", idx + 1, self.len()),
        }
    }
}

/// Split one raw-file line into fields.
///
/// Honours single and double quotes, and treats the first unquoted `/` as the start of a
/// trailing comment (which is where section markers hide their names).
pub fn split_fields(line: &str) -> (Vec<Field>, Option<String>) {
    let mut fields = Vec::new();
    let mut comment = None;
    let mut current = String::new();
    let mut quoted_field = false;
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    quoted_field = true;
                }
                '/' => {
                    comment = Some(line[i + 1..].trim().to_string());
                    break;
                }
                ',' => {
                    fields.push(finish_field(&current, quoted_field));
                    current.clear();
                    quoted_field = false;
                }
                _ => current.push(c),
            },
        }
    }

    // A line is a record only if something preceded the comment; `0 /END OF...` yields ["0"].
    if !current.trim().is_empty() || quoted_field || !fields.is_empty() {
        fields.push(finish_field(&current, quoted_field));
    }
    (fields, comment)
}

fn finish_field(text: &str, quoted: bool) -> Field {
    if quoted {
        // Quoted values are fixed-width and padded; trailing blanks are never significant.
        Field {
            text: text.trim_end().to_string(),
            quoted: true,
        }
    } else {
        Field::unquoted(text)
    }
}

/// Which data section a record belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    SystemWide,
    Bus,
    Load,
    FixedShunt,
    Generator,
    Branch,
    Transformer,
    SwitchedShunt,
    /// A section we recognise the name of but do not read.
    Other(String),
    /// Data following a marker that did not name what comes next.
    Unknown,
}

impl Section {
    pub fn name(&self) -> &str {
        match self {
            Section::SystemWide => "SYSTEM-WIDE",
            Section::Bus => "BUS",
            Section::Load => "LOAD",
            Section::FixedShunt => "FIXED SHUNT",
            Section::Generator => "GENERATOR",
            Section::Branch => "BRANCH",
            Section::Transformer => "TRANSFORMER",
            Section::SwitchedShunt => "SWITCHED SHUNT",
            Section::Other(name) => name,
            Section::Unknown => "UNKNOWN",
        }
    }

    fn from_name(name: &str) -> Section {
        match name {
            "SYSTEM-WIDE" => Section::SystemWide,
            "BUS" => Section::Bus,
            "LOAD" => Section::Load,
            "FIXED SHUNT" => Section::FixedShunt,
            "GENERATOR" => Section::Generator,
            "BRANCH" => Section::Branch,
            "TRANSFORMER" => Section::Transformer,
            "SWITCHED SHUNT" => Section::SwitchedShunt,
            other => Section::Other(other.to_string()),
        }
    }
}

/// Section order for revisions that write bare `0` terminators without naming what follows.
const ORDER_V33: &[&str] = &[
    "BUS",
    "LOAD",
    "FIXED SHUNT",
    "GENERATOR",
    "BRANCH",
    "TRANSFORMER",
    "AREA",
    "TWO-TERMINAL DC",
    "VSC DC LINE",
    "IMPEDANCE CORRECTION",
    "MULTI-TERMINAL DC",
    "MULTI-SECTION LINE",
    "ZONE",
    "INTER-AREA TRANSFER",
    "OWNER",
    "FACTS DEVICE",
    "SWITCHED SHUNT",
    "GNE DEVICE",
    "INDUCTION MACHINE",
];

const ORDER_V35: &[&str] = &[
    "SYSTEM-WIDE",
    "BUS",
    "LOAD",
    "FIXED SHUNT",
    "GENERATOR",
    "BRANCH",
    "SYSTEM SWITCHING DEVICE",
    "TRANSFORMER",
    "AREA",
    "TWO-TERMINAL DC",
    "VSC DC LINE",
    "IMPEDANCE CORRECTION",
    "MULTI-TERMINAL DC",
    "MULTI-SECTION LINE",
    "ZONE",
    "INTER-AREA TRANSFER",
    "OWNER",
    "FACTS DEVICE",
    "SWITCHED SHUNT",
    "GNE DEVICE",
    "INDUCTION MACHINE",
    "SUBSTATION",
];

/// Extract the section name a marker comment announces.
///
/// `END OF BUS DATA, BEGIN LOAD DATA` names its successor; `END OF SWITCHED SHUNT DATA` does
/// not, even though more sections follow it.
fn section_after_marker(comment: &str) -> Option<String> {
    let upper = comment.to_ascii_uppercase();
    let begin = upper.find("BEGIN")? + "BEGIN".len();
    let rest = upper[begin..].trim();
    let name = rest.strip_suffix("DATA").unwrap_or(rest).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The three free-text lines and the case-wide values that open every raw file.
#[derive(Debug, Clone, Default)]
pub struct CaseHeader {
    pub ic: i32,
    pub sbase: f64,
    pub rev: i32,
    pub basfrq: f64,
    pub comments: Vec<String>,
}

/// Pulls records out of a raw file one at a time, tracking which section each belongs to.
pub struct Scanner<'a> {
    lines: Vec<&'a str>,
    idx: usize,
    section: Section,
    rev: i32,
    finished: bool,
}

impl<'a> Scanner<'a> {
    /// Read the file header and position the scanner at the first data record.
    pub fn new(text: &'a str) -> Result<(CaseHeader, Scanner<'a>), PsseError> {
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        let mut idx = 0;

        // `@!` lines are column-header comments; they precede the header record too.
        while idx < lines.len() && is_ignorable(lines[idx]) {
            idx += 1;
        }
        let header_line = lines.get(idx).ok_or(PsseError::Truncated {
            line: idx + 1,
            what: "the case header".to_string(),
        })?;
        let (fields, _) = split_fields(header_line);
        let header_rec = Record {
            fields,
            comment: None,
            line: idx + 1,
        };
        let rev = header_rec.opt_i32(2, 33);
        if !(32..=35).contains(&rev) {
            return Err(PsseError::UnsupportedRevision(rev));
        }
        let header = CaseHeader {
            ic: header_rec.opt_i32(0, 0),
            sbase: header_rec.opt_f64(1, 100.0),
            rev,
            basfrq: header_rec.opt_f64(5, 60.0),
            comments: lines
                .iter()
                .skip(idx + 1)
                .take(2)
                .map(|l| l.trim().to_string())
                .collect(),
        };
        idx += 3; // header record plus its two free-text comment lines

        let section = if rev >= 35 {
            Section::SystemWide
        } else {
            Section::Bus
        };
        Ok((
            header,
            Scanner {
                lines,
                idx,
                section,
                rev,
                finished: false,
            },
        ))
    }

    /// The next data record, or `None` at end of file.
    ///
    /// Section markers are consumed internally; callers only ever see data.
    pub fn next_record(&mut self) -> Result<Option<(Section, Record)>, PsseError> {
        while !self.finished && self.idx < self.lines.len() {
            let line = self.lines[self.idx];
            let line_no = self.idx + 1;
            self.idx += 1;

            if is_ignorable(line) {
                continue;
            }
            if line.trim().eq_ignore_ascii_case("q") {
                self.finished = true;
                break;
            }

            let (fields, comment) = split_fields(line);
            let record = Record {
                fields,
                comment,
                line: line_no,
            };
            if record.fields.is_empty() {
                continue;
            }
            if record.is_terminator() {
                self.advance_section(record.comment.as_deref());
                continue;
            }
            return Ok(Some((self.section.clone(), record)));
        }
        Ok(None)
    }

    /// Pull a continuation line of a multi-line record, such as a transformer's later windings.
    pub fn continuation(&mut self, section: &Section, what: &str) -> Result<Record, PsseError> {
        match self.next_record()? {
            Some((s, rec)) if &s == section => Ok(rec),
            Some((_, rec)) => Err(PsseError::Truncated {
                line: rec.line,
                what: what.to_string(),
            }),
            None => Err(PsseError::Truncated {
                line: self.lines.len(),
                what: what.to_string(),
            }),
        }
    }

    fn advance_section(&mut self, comment: Option<&str>) {
        self.section = match comment.and_then(section_after_marker) {
            Some(name) => Section::from_name(&name),
            // Nothing named; fall back to the canonical order for this revision.
            None => self.next_section_positional(),
        };
    }

    fn next_section_positional(&self) -> Section {
        let order = if self.rev >= 35 { ORDER_V35 } else { ORDER_V33 };
        match order.iter().position(|n| *n == self.section.name()) {
            Some(i) => order
                .get(i + 1)
                .map(|n| Section::from_name(n))
                .unwrap_or(Section::Unknown),
            None => Section::Unknown,
        }
    }
}

/// Blank lines and `@!` column headers carry no data.
fn is_ignorable(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with("@!")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        split_fields(line).0.into_iter().map(|f| f.text).collect()
    }

    #[test]
    fn splits_plain_fields() {
        assert_eq!(texts("  137,'1 ',1,   4"), ["137", "1", "1", "4"]);
    }

    #[test]
    fn keeps_commas_and_slashes_inside_quotes() {
        assert_eq!(
            texts("1,'Smith, J/K  ', 138.0"),
            ["1", "Smith, J/K", "138.0"]
        );
    }

    #[test]
    fn strips_trailing_comment() {
        let (fields, comment) = split_fields("0,    100.00, 35, 0, 0, 60.00       / March 07");
        assert_eq!(fields.len(), 6);
        assert_eq!(comment.as_deref(), Some("March 07"));
    }

    #[test]
    fn handles_double_quoted_fields() {
        let (fields, _) = split_fields(r#"RATING, 1, "      ", "        ""#);
        assert_eq!(fields.len(), 4);
        assert!(fields[2].quoted && fields[2].text.is_empty());
    }

    #[test]
    fn terminator_has_one_field() {
        let (fields, comment) = split_fields("0 / END OF BUS DATA, BEGIN LOAD DATA");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].text, "0");
        assert_eq!(
            section_after_marker(&comment.unwrap()).as_deref(),
            Some("LOAD")
        );
    }

    #[test]
    fn marker_without_begin_names_nothing() {
        assert_eq!(section_after_marker("END OF SWITCHED SHUNT DATA"), None);
    }

    #[test]
    fn data_line_starting_with_zero_is_not_a_terminator() {
        // A transformer impedance line can legitimately open with a bare zero.
        let (fields, _) = split_fields("0, 0.1126, 100.00");
        assert_eq!(fields.len(), 3);
        let rec = Record {
            fields,
            comment: None,
            line: 1,
        };
        assert!(!rec.is_terminator());
    }

    #[test]
    fn decodes_cp1252_bytes_without_error() {
        let decoded = decode_latin1(b"quote \x93here\x94");
        assert!(decoded.starts_with("quote "));
        // Every byte becomes exactly one char, including the two CP-1252 curly quotes.
        assert_eq!(decoded.chars().count(), 12);
    }
}
