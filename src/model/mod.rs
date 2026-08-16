//! The network as the diagram needs to see it.
//!
//! [`Network`] wraps a parsed [`RawCase`] with the two lookups the drawing workflow depends on:
//! what connects to a given bus (so a diagram can be grown outward from one), and what equipment
//! hangs off it (so the bus can be drawn with its generators, loads and shunts).

use std::collections::{BTreeMap, BTreeSet};

use crate::psse::raw::{
    Branch, Bus, FixedShunt, Generator, Load, RawCase, SwitchedShunt, Transformer,
};

/// A branch or transformer, identified by its index in the network's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElemId {
    Branch(usize),
    Transformer(usize),
}

/// Borrowed view of whichever element an [`ElemId`] names.
#[derive(Debug, Clone, Copy)]
pub enum Element<'a> {
    Branch(&'a Branch),
    Transformer(&'a Transformer),
}

impl Element<'_> {
    pub fn terminals(&self) -> Vec<i32> {
        match self {
            Element::Branch(b) => vec![b.from, b.to],
            Element::Transformer(t) => t.terminals(),
        }
    }

    pub fn ckt(&self) -> &str {
        match self {
            Element::Branch(b) => &b.ckt,
            Element::Transformer(t) => &t.ckt,
        }
    }

    pub fn in_service(&self) -> bool {
        match self {
            Element::Branch(b) => b.status != 0,
            Element::Transformer(t) => t.status != 0,
        }
    }
}

/// Indices of the equipment connected to one bus.
#[derive(Debug, Clone, Default)]
pub struct Attached {
    pub generators: Vec<usize>,
    pub loads: Vec<usize>,
    pub fixed_shunts: Vec<usize>,
    pub switched_shunts: Vec<usize>,
}

impl Attached {
    /// How many symbols will hang off the bus bar, which sets how long to draw it.
    pub fn count(&self) -> usize {
        self.generators.len()
            + self.loads.len()
            + self.fixed_shunts.len()
            + self.switched_shunts.len()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Network {
    pub sbase: f64,
    pub rev: i32,
    /// `IC = 1` marks a change case, whose records modify a case held elsewhere rather than
    /// define a whole network, so the topology read from it is expected to be incomplete.
    pub change_case: bool,
    pub buses: BTreeMap<i32, Bus>,
    pub branches: Vec<Branch>,
    pub transformers: Vec<Transformer>,
    pub generators: Vec<Generator>,
    pub loads: Vec<Load>,
    pub fixed_shunts: Vec<FixedShunt>,
    pub switched_shunts: Vec<SwitchedShunt>,
    /// Sections the reader skipped, for display.
    pub skipped: BTreeMap<String, usize>,
    /// Records that named a bus the case does not define, for display.
    pub dangling: usize,

    incidence: BTreeMap<i32, Vec<ElemId>>,
    attached: BTreeMap<i32, Attached>,
    /// Terminal pairs carrying more than one element, so circuit IDs can be drawn only where
    /// they disambiguate something.
    parallel: BTreeSet<(i32, i32)>,
}

impl Network {
    pub fn from_raw(case: RawCase) -> Network {
        let buses: BTreeMap<i32, Bus> = case.buses.into_iter().map(|b| (b.number, b)).collect();
        let mut dangling = 0;

        // Records pointing at buses the case never defined cannot be drawn; drop them rather
        // than carry references that every consumer would have to re-check.
        let known = |n: &i32| buses.contains_key(n);
        let branches: Vec<Branch> = case
            .branches
            .into_iter()
            .filter(|b| {
                let ok = known(&b.from) && known(&b.to);
                dangling += usize::from(!ok);
                ok
            })
            .collect();
        let transformers: Vec<Transformer> = case
            .transformers
            .into_iter()
            .filter(|t| {
                let ok = t.terminals().iter().all(known);
                dangling += usize::from(!ok);
                ok
            })
            .collect();

        let mut net = Network {
            sbase: case.header.sbase,
            rev: case.header.rev,
            change_case: case.header.ic != 0,
            buses,
            branches,
            transformers,
            generators: case.generators,
            loads: case.loads,
            fixed_shunts: case.fixed_shunts,
            switched_shunts: case.switched_shunts,
            skipped: case.skipped,
            dangling,
            ..Default::default()
        };
        net.build_indices();
        net
    }

    fn build_indices(&mut self) {
        let mut pair_counts: BTreeMap<(i32, i32), usize> = BTreeMap::new();

        for i in 0..self.branches.len() {
            self.index_element(ElemId::Branch(i), &mut pair_counts);
        }
        for i in 0..self.transformers.len() {
            self.index_element(ElemId::Transformer(i), &mut pair_counts);
        }
        self.parallel = pair_counts
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(pair, _)| pair)
            .collect();

        // Equipment on a bus the case never defined is dropped for the same reason branches are.
        for (i, g) in self.generators.iter().enumerate() {
            if self.buses.contains_key(&g.bus) {
                self.attached.entry(g.bus).or_default().generators.push(i);
            }
        }
        for (i, l) in self.loads.iter().enumerate() {
            if self.buses.contains_key(&l.bus) {
                self.attached.entry(l.bus).or_default().loads.push(i);
            }
        }
        for (i, s) in self.fixed_shunts.iter().enumerate() {
            if self.buses.contains_key(&s.bus) {
                self.attached.entry(s.bus).or_default().fixed_shunts.push(i);
            }
        }
        for (i, s) in self.switched_shunts.iter().enumerate() {
            if self.buses.contains_key(&s.bus) {
                self.attached
                    .entry(s.bus)
                    .or_default()
                    .switched_shunts
                    .push(i);
            }
        }
    }

    fn index_element(&mut self, id: ElemId, pair_counts: &mut BTreeMap<(i32, i32), usize>) {
        let terminals = self.element(id).terminals();
        for bus in &terminals {
            self.incidence.entry(*bus).or_default().push(id);
        }
        // Three-winding transformers count as parallel on each of their bus pairs.
        for (a, b) in terminal_pairs(&terminals) {
            *pair_counts.entry((a, b)).or_insert(0) += 1;
        }
    }

    pub fn element(&self, id: ElemId) -> Element<'_> {
        match id {
            ElemId::Branch(i) => Element::Branch(&self.branches[i]),
            ElemId::Transformer(i) => Element::Transformer(&self.transformers[i]),
        }
    }

    pub fn bus(&self, number: i32) -> Option<&Bus> {
        self.buses.get(&number)
    }

    /// Every branch and transformer touching this bus. This is what "grow the diagram" walks.
    pub fn incident(&self, bus: i32) -> &[ElemId] {
        self.incidence.get(&bus).map_or(&[], Vec::as_slice)
    }

    pub fn attached(&self, bus: i32) -> Option<&Attached> {
        self.attached.get(&bus)
    }

    /// True when another element shares this one's terminal pair, so its circuit ID matters.
    pub fn has_parallel(&self, id: ElemId) -> bool {
        terminal_pairs(&self.element(id).terminals())
            .into_iter()
            .any(|pair| self.parallel.contains(&pair))
    }

    /// The distinct nominal voltages in the case, descending, for the legend and colour table.
    pub fn voltage_levels(&self) -> Vec<f64> {
        let mut levels: Vec<f64> = self
            .buses
            .values()
            .map(|b| b.base_kv)
            .map(|kv| (kv * 100.0).round() / 100.0)
            .collect();
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup();
        levels
    }

    /// A short human label, e.g. `2-722 ckt 1` or `2-3 XFMR`.
    pub fn label(&self, id: ElemId) -> String {
        let element = self.element(id);
        let terminals = element.terminals();
        let buses: Vec<String> = terminals.iter().map(|b| b.to_string()).collect();
        let kind = match element {
            Element::Branch(_) => "line",
            Element::Transformer(t) if t.is_three_winding() => "3-w xfmr",
            Element::Transformer(t) if t.is_phase_shifter() => "phase shifter",
            Element::Transformer(_) => "xfmr",
        };
        format!("{} {kind} {}", buses.join("-"), element.ckt())
    }
}

/// Unordered bus pairs an element spans; a three-winding transformer spans three.
fn terminal_pairs(terminals: &[i32]) -> Vec<(i32, i32)> {
    let mut pairs = Vec::new();
    for (i, a) in terminals.iter().enumerate() {
        for b in &terminals[i + 1..] {
            pairs.push((*a.min(b), *a.max(b)));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psse;
    use std::path::Path;

    fn sample() -> Network {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/MemphisCase2026_Mar7.RAW");
        Network::from_raw(psse::parse_file(&path).expect("sample case parses"))
    }

    #[test]
    fn indexes_every_element_against_both_terminals() {
        let net = sample();
        let total: usize = net.buses.keys().map(|b| net.incident(*b).len()).sum();
        assert_eq!(total, 2 * (net.branches.len() + net.transformers.len()));
        assert_eq!(net.dangling, 0);
    }

    #[test]
    fn incidence_finds_both_a_line_and_a_transformer() {
        let net = sample();
        let labels: Vec<String> = net.incident(2).iter().map(|id| net.label(*id)).collect();
        assert!(labels.iter().any(|l| l == "2-722 line 1"), "{labels:?}");
        assert!(labels.iter().any(|l| l == "2-3 xfmr 1"), "{labels:?}");
    }

    #[test]
    fn reports_the_voltage_levels_present() {
        let net = sample();
        assert_eq!(net.voltage_levels(), vec![345.0, 138.0, 69.0, 18.0]);
    }

    #[test]
    fn finds_equipment_hanging_off_a_bus() {
        let net = sample();
        let attached = net.attached(808).expect("bus 808 carries a generator");
        assert_eq!(attached.generators.len(), 1);
        assert_eq!(net.generators[attached.generators[0]].bus, 808);
        // Every generator, load and shunt in the case lands on some bus.
        let indexed: usize = net
            .buses
            .keys()
            .filter_map(|b| net.attached(*b))
            .map(Attached::count)
            .sum();
        let expected = net.generators.len() + net.loads.len() + net.switched_shunts.len();
        assert_eq!(indexed, expected);
    }

    #[test]
    fn flags_only_buses_with_more_than_one_circuit() {
        let net = sample();
        let parallel = (0..net.branches.len())
            .filter(|i| net.has_parallel(ElemId::Branch(*i)))
            .count();
        assert!(
            parallel < net.branches.len(),
            "not every branch is parallel"
        );
    }
}
