#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diagram;
mod model;
mod psse;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // A headless read of a case, for checking the reader without opening a window.
        Some("--summary") => {
            match args.get(1) {
                Some(path) => summary(PathBuf::from(path)),
                None => {
                    eprintln!("usage: pf-vis --summary <case.raw>");
                    std::process::exit(2);
                }
            }
            Ok(())
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!(
                "unknown argument {flag:?}\nusage: pf-vis [--summary] [case.raw | drawing.json]"
            );
            std::process::exit(2);
        }
        other => {
            let open = other.map(PathBuf::from);
            eframe::run_native(
                "pf-vis",
                eframe::NativeOptions {
                    viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 950.0]),
                    ..Default::default()
                },
                Box::new(|cc| {
                    // A drawing tool reads as a sheet of paper, not a dark editor.
                    cc.egui_ctx.set_theme(egui::Theme::Light);
                    Ok(Box::new(app::PfVisApp::new(open)))
                }),
            )
        }
    }
}

/// Read a case and print what came out, without involving the GUI.
fn summary(path: PathBuf) {
    let case = match psse::parse_file(&path) {
        Ok(case) => case,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        }
    };

    println!("{}", path.display());
    println!(
        "  revision {}  sbase {} MVA  {} Hz",
        case.header.rev, case.header.sbase, case.header.basfrq
    );
    for comment in case.header.comments.iter().filter(|c| !c.is_empty()) {
        println!("  {comment}");
    }
    println!("  buses            {}", case.buses.len());
    println!("  loads            {}", case.loads.len());
    println!("  fixed shunts     {}", case.fixed_shunts.len());
    println!("  switched shunts  {}", case.switched_shunts.len());
    println!("  generators       {}", case.generators.len());
    println!("  branches         {}", case.branches.len());
    let three = case
        .transformers
        .iter()
        .filter(|t| t.is_three_winding())
        .count();
    println!(
        "  transformers     {} ({} two-winding, {} three-winding)",
        case.transformers.len(),
        case.transformers.len() - three,
        three
    );
    if !case.skipped.is_empty() {
        let skipped: Vec<String> = case
            .skipped
            .iter()
            .map(|(name, n)| format!("{name} ({n})"))
            .collect();
        println!("  not read         {}", skipped.join(", "));
    }
}
