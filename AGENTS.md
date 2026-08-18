# AGENTS.md

This document provides context, architectural reference, build/test commands, and development guidelines for AI agents working in the `pf-vis` repository.

---

## Project Overview

`pf-vis` is a lightweight, fast native desktop application written in Rust for inspecting PSS/E power flow cases (`.RAW` files) and dynamically building vector one-line diagrams.

- **GUI Framework**: Built with [`egui`](https://github.com/emilk/egui) / [`eframe`](https://github.com/emilk/egui/tree/master/crates/eframe) `0.36` using the 2D `glow` OpenGL backend.
- **Key Capabilities**:
  - PSS/E `.RAW` parser supporting Revisions 32–35 (Buses, Branches, Transformers, Generators, Loads, Fixed/Switched Shunts).
  - Infinite vector drawing sheet with smooth pan and anchored zoom.
  - Incremental diagram building (grow network from any busbar along incident lines and transformers).
  - Orthogonal line routing and collision-aware automatic grid placement.
  - Voltage-level color coding with logarithmic ratio matching for non-standard voltages.
  - Headless CLI summary mode for batch inspection without launching the UI.
  - Project file (`.json`) serialization preserving relative paths to case files.

---

## Development Workflow & Common Commands

All commands should be executed from the repository root.

### Build & Check
```bash
# Verify compilation
cargo check

# Compile in debug mode
cargo build

# Compile in release mode (optimized)
cargo build --release

# Run linter
cargo clippy
```

### Testing
```bash
# Run all unit tests
cargo test

# Run a specific test by name
cargo test <test_name_filter>
```

### Running the Application
```bash
# Launch GUI (empty state / file picker ready)
cargo run

# Launch GUI directly with a sample PSS/E case
cargo run -- cases/MemphisCase2026_Mar7.RAW

# Launch GUI with a saved diagram project
cargo run -- path/to/project.json

# Headless case summary (no GUI launched)
cargo run -- --summary cases/MemphisCase2026_Mar7.RAW
```

---

## Architecture & Codebase Map

```
pf-vis/
├── cases/                  # Sample PSS/E RAW power flow cases (e.g., MemphisCase2026_Mar7.RAW)
├── docs/                   # Documentation and visual screenshots
├── src/
│   ├── main.rs             # CLI handling, summary mode, and eframe native app initialization
│   ├── app.rs              # Main egui GUI application logic, UI panels, canvas integration, inspector
│   ├── model/
│   │   └── mod.rs          # NetworkModel: graph indexing, bus incidence, connected equipment lookup
│   ├── psse/
│   │   ├── mod.rs          # Parser entry points (parse_file, parse_str)
│   │   ├── raw.rs          # Typed record structs (Bus, Branch, Transformer, Generator, Load, Shunts)
│   │   └── scan.rs         # Low-level tokenizer/lexer handling CP-1252, quoted fields, comments, delimiters
│   └── diagram/
│       ├── mod.rs          # DiagramState, PlacedBus, PlacedBranch, JSON project persistence
│       ├── layout.rs       # Grid placement, incremental network expansion, orthogonal route planning
│       ├── render.rs       # egui painting routines for busbars, branches, transformer coils, equipment
│       └── style.rs        # Voltage color palettes, log-ratio level mapping, diagram typography & metrics
├── Cargo.toml              # Dependencies and crate configuration (Rust 2024 edition)
├── README.md               # User-facing project documentation
└── USER_GUIDE.md           # Step-by-step visual user manual
```

---

## Key Modules & Responsibilities

### 1. `src/psse/` — PSS/E Parser
- `scan.rs`: Tokenizes raw PSS/E format. Handles comma/space separated fields, single/double quotes, slash comments (`/`), section terminators (`0 /` or `0`), and CP1252 character encodings.
- `raw.rs`: Constructs typed Rust structs (`RawCase`, `BusRecord`, `BranchRecord`, `TransformerRecord`, etc.). Accurately handles differences across revisions (such as Revision 35 column offsets for branch names and generator `NREG`).

### 2. `src/model/` — Network Graph Representation
- `NetworkModel`: Wraps `RawCase` with bi-directional spatial indexes (`HashMap<i32, Vec<IncidentElement>>`).
- Fast lookup for equipment (generators, loads, shunts) connected to any bus.
- Identifies multi-circuit branches and parallel connections.

### 3. `src/diagram/` — Diagram State, Layout & Rendering
- `DiagramState`: Represents the active drawing sheet (placed buses, routed branches, equipment visibility, transform matrix).
- `layout.rs`: Implements geometric placement, grid snapping, collision clearance, and orthogonal routing between busbars.
- `render.rs`: Renders vector elements onto the `egui::Painter`.
- `style.rs`: Formats voltage labels and assigns distinct colors based on nominal kV ratings.

### 4. `src/app.rs` — Interactive GUI
- Top menu bar (Open Case, Open Project, Save Project, View controls).
- Central panel canvas with pan/zoom viewport math.
- Side panels: Bus search/palette, Network hierarchy, and Element Inspector for selected items.

---

## Coding Conventions & Guidelines

1. **Rust Edition & Idioms**: Use Rust 2024 edition idioms. Write clear, expressive code with appropriate error handling (`Result`/`Option`).
2. **Resilient Parsing**: Never crash on malformed non-critical records or unexpected fields in PSS/E cases. Collect parse warnings and report skipped sections gracefully.
3. **Coordinate Separation**: Keep canvas world coordinates (continuous mathematical space) strictly separated from egui screen coordinates (pixels). Use the camera transformation helpers in `layout.rs`/`app.rs`.
4. **Drawing Aesthetics**: The application defaults to a clean, high-contrast light theme representing an engineering drawing sheet. Maintain consistent line weights, font sizing, and voltage color palettes.
5. **Testing**: When modifying layout algorithms or parser logic, ensure existing unit tests pass (`cargo test`) and add unit tests covering new edge cases.
