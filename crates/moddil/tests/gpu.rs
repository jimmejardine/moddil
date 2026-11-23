//! The Mandelbulb example's kernel lowers to WGSL that naga validates (`gpu.kernel.1`).

use std::path::Path;

use moddil_native::layout::{Layouts, Shape};
use moddil_sema::{Database, ModInfo, Program, ProgramFile, SourceFile, Ty};

fn program(db: &Database, path: &Path) -> Program {
    let project = moddil_project::load(path).expect("the example loads");
    let files = project
        .files
        .iter()
        .map(|f| ProgramFile {
            source: SourceFile::new(db, f.display.clone(), f.text.clone()),
            mod_: f.mod_,
            module: f.module.clone(),
            exe: f.exe,
        })
        .collect();
    let mods = project
        .mods
        .iter()
        .map(|m| ModInfo {
            name: m.name.clone(),
            deps: m.deps.clone(),
            stub: m.stub,
            unsafe_ok: m.unsafe_ok,
            os_ok: m.os_ok,
        })
        .collect();
    Program::new(db, files, mods)
}

#[test]
fn the_mandelbulb_kernel_lowers_to_valid_wgsl() {
    let db = Database::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mandelbulb/web");
    let program = program(&db, &root);
    let items = moddil_sema::item_tree(&db, program);
    let kernel = items
        .fns
        .iter()
        .position(|f| f.kernel && f.name == "render")
        .expect("the kernel `render`") as u32;
    let mut layouts = Layouts::new(items);
    let mut offsets = |t: &Ty| match &layouts.of(t).ok()?.shape {
        Shape::Fields(fs) => Some(fs.iter().map(|(o, _)| u64::from(*o)).collect()),
        _ => None,
    };
    let wgsl =
        moddil_gpu::wgsl(&db, program, kernel, &mut offsets).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        wgsl.contains("@compute @workgroup_size(64, 1, 1)"),
        "{wgsl}"
    );
}
