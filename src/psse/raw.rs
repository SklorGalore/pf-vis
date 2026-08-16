//! Typed records read out of a PSS/E raw file.
//!
//! Only the sections a one-line diagram needs are decoded: bus, load, fixed shunt, generator,
//! branch, transformer and switched shunt. Everything else is counted and reported so nothing
//! is dropped silently.
//!
//! Field positions moved between revisions. Where the shift can be detected from the record
//! itself (a quoted branch name, a switched shunt's `RMIDNT`) the parser looks rather than
//! guesses; elsewhere it keys off `REV`. Revision 35 is verified against a real case; 32-34
//! are best effort.

use std::collections::BTreeMap;

pub use super::scan::CaseHeader as Header;
use super::scan::{PsseError, Record, Scanner, Section};

#[derive(Debug, Clone)]
pub struct Bus {
    pub number: i32,
    pub name: String,
    pub base_kv: f64,
    /// 1 = load bus, 2 = generator bus, 3 = swing, 4 = isolated.
    pub ide: i32,
    pub area: i32,
    pub zone: i32,
    pub vm: f64,
    pub va: f64,
}

#[derive(Debug, Clone)]
pub struct Load {
    pub bus: i32,
    pub id: String,
    pub status: i32,
    pub pl: f64,
    pub ql: f64,
}

#[derive(Debug, Clone)]
pub struct FixedShunt {
    pub bus: i32,
    pub id: String,
    pub status: i32,
    pub gl: f64,
    pub bl: f64,
}

#[derive(Debug, Clone)]
pub struct Generator {
    pub bus: i32,
    pub id: String,
    pub pg: f64,
    pub qg: f64,
    pub mbase: f64,
    pub status: i32,
}

#[derive(Debug, Clone)]
pub struct Branch {
    pub from: i32,
    pub to: i32,
    pub ckt: String,
    pub r: f64,
    pub x: f64,
    pub b: f64,
    pub rate_a: f64,
    pub status: i32,
    pub name: String,
}

/// One transformer winding's tap, nominal voltage, phase shift and rating.
#[derive(Debug, Clone)]
pub struct Winding {
    pub windv: f64,
    pub nomv: f64,
    pub ang: f64,
    pub rate_a: f64,
}

#[derive(Debug, Clone)]
pub struct Transformer {
    pub i: i32,
    pub j: i32,
    /// Third winding bus, or 0 for a two-winding transformer.
    pub k: i32,
    pub ckt: String,
    pub name: String,
    pub status: i32,
    pub r12: f64,
    pub x12: f64,
    pub windings: Vec<Winding>,
}

impl Transformer {
    pub fn is_three_winding(&self) -> bool {
        self.k != 0
    }

    /// The buses this transformer connects, in winding order.
    pub fn terminals(&self) -> Vec<i32> {
        if self.is_three_winding() {
            vec![self.i, self.j, self.k]
        } else {
            vec![self.i, self.j]
        }
    }

    /// Nonzero phase shift on any winding marks a phase shifting transformer.
    pub fn is_phase_shifter(&self) -> bool {
        self.windings.iter().any(|w| w.ang.abs() > 1e-6)
    }
}

#[derive(Debug, Clone)]
pub struct SwitchedShunt {
    pub bus: i32,
    pub id: String,
    pub status: i32,
    /// Initial susceptance in Mvar at 1.0 pu; positive is capacitive, negative reactive.
    pub binit: f64,
}

/// Everything read out of one raw file.
#[derive(Debug, Clone, Default)]
pub struct RawCase {
    pub header: Header,
    pub buses: Vec<Bus>,
    pub loads: Vec<Load>,
    pub fixed_shunts: Vec<FixedShunt>,
    pub generators: Vec<Generator>,
    pub branches: Vec<Branch>,
    pub transformers: Vec<Transformer>,
    pub switched_shunts: Vec<SwitchedShunt>,
    /// Sections present in the file that this reader does not decode, with record counts.
    pub skipped: BTreeMap<String, usize>,
}

pub fn parse(text: &str) -> Result<RawCase, PsseError> {
    let (header, mut scanner) = Scanner::new(text)?;
    let rev = header.rev;
    let mut case = RawCase {
        header,
        ..Default::default()
    };

    while let Some((section, rec)) = scanner.next_record()? {
        match section {
            Section::Bus => case.buses.push(parse_bus(&rec)?),
            Section::Load => case.loads.push(parse_load(&rec)?),
            Section::FixedShunt => case.fixed_shunts.push(parse_fixed_shunt(&rec)?),
            Section::Generator => case.generators.push(parse_generator(&rec, rev)?),
            Section::Branch => case.branches.push(parse_branch(&rec)?),
            Section::Transformer => case
                .transformers
                .push(parse_transformer(&rec, &mut scanner)?),
            Section::SwitchedShunt => case.switched_shunts.push(parse_switched_shunt(&rec)?),
            other => *case.skipped.entry(other.name().to_string()).or_insert(0) += 1,
        }
    }
    Ok(case)
}

fn parse_bus(rec: &Record) -> Result<Bus, PsseError> {
    Ok(Bus {
        number: rec.i32(0)?,
        name: rec.text(1)?.trim().to_string(),
        base_kv: rec.f64(2)?,
        ide: rec.opt_i32(3, 1),
        area: rec.opt_i32(4, 1),
        zone: rec.opt_i32(5, 1),
        vm: rec.opt_f64(7, 1.0),
        va: rec.opt_f64(8, 0.0),
    })
}

fn parse_load(rec: &Record) -> Result<Load, PsseError> {
    Ok(Load {
        bus: rec.i32(0)?,
        id: rec.text(1)?.trim().to_string(),
        status: rec.opt_i32(2, 1),
        pl: rec.opt_f64(5, 0.0),
        ql: rec.opt_f64(6, 0.0),
    })
}

fn parse_fixed_shunt(rec: &Record) -> Result<FixedShunt, PsseError> {
    Ok(FixedShunt {
        bus: rec.i32(0)?,
        id: rec.text(1)?.trim().to_string(),
        status: rec.opt_i32(2, 1),
        gl: rec.opt_f64(3, 0.0),
        bl: rec.opt_f64(4, 0.0),
    })
}

/// Revision 35 inserts `NREG` after `IREG`, shifting everything from `MBASE` on by one.
///
/// The offset is confirmed against the record before use: status is always 0 or 1, so a value
/// outside that range means the assumed layout is wrong and the other one is tried.
fn generator_shift(rec: &Record, rev: i32) -> usize {
    let assumed = if rev >= 35 { 1 } else { 0 };
    let plausible = |shift: usize| matches!(rec.opt_i32(14 + shift, -1), 0 | 1);
    if plausible(assumed) {
        assumed
    } else if plausible(1 - assumed) {
        1 - assumed
    } else {
        assumed
    }
}

fn parse_generator(rec: &Record, rev: i32) -> Result<Generator, PsseError> {
    let shift = generator_shift(rec, rev);
    Ok(Generator {
        bus: rec.i32(0)?,
        id: rec.text(1)?.trim().to_string(),
        pg: rec.opt_f64(2, 0.0),
        qg: rec.opt_f64(3, 0.0),
        mbase: rec.opt_f64(8 + shift, 100.0),
        status: rec.opt_i32(14 + shift, 1),
    })
}

/// Revisions 34 and up carry a quoted branch name after `B` and twelve ratings instead of three.
///
/// The name is quoted and the ratings are not, so the record says which layout it is.
fn parse_branch(rec: &Record) -> Result<Branch, PsseError> {
    let named = rec.fields.get(6).is_some_and(|f| f.quoted);
    let (name, rate_idx, stat_idx) = if named {
        (rec.opt_text(6).trim().to_string(), 7, 23)
    } else {
        (String::new(), 6, 13)
    };
    Ok(Branch {
        from: rec.i32(0)?,
        to: rec.i32(1)?,
        ckt: rec.text(2)?.trim().to_string(),
        r: rec.opt_f64(3, 0.0),
        x: rec.f64(4)?,
        b: rec.opt_f64(5, 0.0),
        rate_a: rec.opt_f64(rate_idx, 0.0),
        status: rec.opt_i32(stat_idx, 1),
        name,
    })
}

/// Transformers span four lines (two winding) or five (three winding).
fn parse_transformer(first: &Record, scanner: &mut Scanner) -> Result<Transformer, PsseError> {
    let k = first.opt_i32(2, 0);
    let three_winding = k != 0;
    let what = "a transformer record";

    let impedance = scanner.continuation(&Section::Transformer, what)?;
    let w1 = scanner.continuation(&Section::Transformer, what)?;
    let w2 = scanner.continuation(&Section::Transformer, what)?;

    let mut windings = vec![parse_winding(&w1), parse_winding(&w2)];
    if three_winding {
        let w3 = scanner.continuation(&Section::Transformer, what)?;
        windings.push(parse_winding(&w3));
    }

    Ok(Transformer {
        i: first.i32(0)?,
        j: first.i32(1)?,
        k,
        ckt: first.text(3)?.trim().to_string(),
        name: first.opt_text(10).trim().to_string(),
        status: first.opt_i32(11, 1),
        r12: impedance.opt_f64(0, 0.0),
        x12: impedance.opt_f64(1, 0.0),
        windings,
    })
}

/// A winding line opens with tap, nominal kV, phase shift and first rating in every revision.
///
/// The second winding of a two-winding transformer stops after the nominal voltage.
fn parse_winding(rec: &Record) -> Winding {
    Winding {
        windv: rec.opt_f64(0, 1.0),
        nomv: rec.opt_f64(1, 0.0),
        ang: rec.opt_f64(2, 0.0),
        rate_a: rec.opt_f64(3, 0.0),
    }
}

/// Switched shunts gained an `ID` in revision 35 and a `NREG` before `RMPCT`, so the leading
/// fields cannot be counted on. `RMIDNT` is the record's last quoted field, and `BINIT` always
/// follows it.
fn parse_switched_shunt(rec: &Record) -> Result<SwitchedShunt, PsseError> {
    let has_id = rec.fields.get(1).is_some_and(|f| f.quoted);
    let (id, status_idx) = if has_id {
        (rec.opt_text(1).trim().to_string(), 4)
    } else {
        (String::new(), 3)
    };
    // The last quoted field is RMIDNT; when it is absent fall back to the revision-33 slot.
    let binit_idx = match rec.last_quoted() {
        Some(rmidnt) if rmidnt > status_idx => rmidnt + 1,
        _ => 9,
    };
    Ok(SwitchedShunt {
        bus: rec.i32(0)?,
        id,
        status: rec.opt_i32(status_idx, 1),
        binit: rec.opt_f64(binit_idx, 0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psse::scan::decode_latin1;
    use std::path::Path;

    fn sample() -> RawCase {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/MemphisCase2026_Mar7.RAW");
        let bytes = std::fs::read(&path).expect("sample case is checked in");
        parse(&decode_latin1(&bytes)).expect("sample case parses")
    }

    #[test]
    fn reads_case_header() {
        let case = sample();
        assert_eq!(case.header.rev, 35);
        assert_eq!(case.header.sbase, 100.0);
        assert_eq!(case.header.basfrq, 60.0);
    }

    #[test]
    fn reads_every_section_completely() {
        let case = sample();
        assert_eq!(case.buses.len(), 993);
        assert_eq!(case.loads.len(), 530);
        assert_eq!(case.fixed_shunts.len(), 0);
        assert_eq!(case.generators.len(), 188);
        assert_eq!(case.branches.len(), 975);
        assert_eq!(case.transformers.len(), 407);
        assert_eq!(case.switched_shunts.len(), 61);
        assert!(case.transformers.iter().all(|t| !t.is_three_winding()));
    }

    #[test]
    fn reads_bus_fields() {
        let case = sample();
        let bus = case.buses.iter().find(|b| b.number == 2).unwrap();
        assert_eq!(bus.name, "Greers Fer~1");
        assert_eq!(bus.base_kv, 138.0);
        assert_eq!(bus.area, 2);
        assert!((bus.vm - 1.0177).abs() < 1e-9);
    }

    #[test]
    fn reads_branch_fields_with_v35_name_column() {
        let case = sample();
        let br = case
            .branches
            .iter()
            .find(|b| b.from == 2 && b.to == 722)
            .unwrap();
        assert_eq!(br.ckt, "1");
        assert!((br.r - 8.55e-3).abs() < 1e-12);
        assert!((br.x - 5.2937e-2).abs() < 1e-12);
        assert!((br.rate_a - 187.10).abs() < 1e-9);
        assert_eq!(br.status, 1);
    }

    #[test]
    fn reads_two_winding_transformer_across_its_four_lines() {
        let case = sample();
        let xf = case
            .transformers
            .iter()
            .find(|t| t.i == 2 && t.j == 3)
            .unwrap();
        assert!(!xf.is_three_winding());
        assert!((xf.x12 - 1.1262e-1).abs() < 1e-12);
        assert_eq!(xf.windings.len(), 2);
        assert_eq!(xf.windings[0].nomv, 138.0);
        assert_eq!(xf.windings[1].nomv, 69.0);
        assert!(!xf.is_phase_shifter());
    }

    #[test]
    fn reads_generator_status_past_the_v35_nreg_column() {
        let case = sample();
        let machine = case.generators.iter().find(|g| g.bus == 808).unwrap();
        assert_eq!(machine.status, 1);
        assert!((machine.mbase - 53.3).abs() < 1e-9);
        assert!((machine.pg - 48.0).abs() < 1e-9);
        assert!(case.generators.iter().all(|g| matches!(g.status, 0 | 1)));
    }

    #[test]
    fn locates_switched_shunt_binit_by_its_rmidnt_anchor() {
        let case = sample();
        let sh = case.switched_shunts.iter().find(|s| s.bus == 5).unwrap();
        assert_eq!(sh.status, 1);
        assert!((sh.binit - 61.05).abs() < 1e-9);
    }

    #[test]
    fn reports_sections_it_does_not_read() {
        let case = sample();
        // The sample carries twelve RATING records plus solver settings before the bus data.
        assert!(case.skipped.contains_key("SYSTEM-WIDE"));
        assert!(!case.skipped.contains_key("BUS"));
    }
}
