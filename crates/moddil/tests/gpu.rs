//! The Mandelbulb example's kernel lowers to WGSL that naga validates (`gpu.kernel.1`).

use std::path::{Path, PathBuf};

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

/// The WGSL of kernel `name` of the program at `root`.
fn wgsl(root: &Path, name: &str) -> String {
    let db = Database::default();
    let program = program(&db, root);
    let items = moddil_sema::item_tree(&db, program);
    let kernel = items
        .fns
        .iter()
        .position(|f| f.kernel && f.name == name)
        .unwrap_or_else(|| panic!("the kernel `{name}`")) as u32;
    let mut layouts = Layouts::new(items);
    let mut offsets = |t: &Ty| match &layouts.of(t).ok()?.shape {
        Shape::Fields(fs) => Some(fs.iter().map(|(o, _)| u64::from(*o)).collect()),
        // An array's elements, one stride apart.
        Shape::Array { stride, len, .. } => Some(
            (0..u64::from(*len))
                .map(|i| i * u64::from(*stride))
                .collect(),
        ),
        _ => None,
    };
    moddil_gpu::wgsl(&db, program, kernel, &mut offsets).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_mandelbulb_kernel_lowers_to_valid_wgsl() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mandelbulb/web");
    let wgsl = wgsl(&root, "render");
    // A workgroup per tile, a thread per pixel.
    assert!(wgsl.contains("@compute @workgroup_size(8, 8, 1)"), "{wgsl}");
    // Real loops and branches: no function needed the block dispatcher.
    assert!(!wgsl.contains("switch"), "{wgsl}");
    assert!(wgsl.contains("loop {") && wgsl.contains("break;"), "{wgsl}");
}

/// A kernel whose helper breaks out of two loops at once, which WGSL's `break` cannot.
const TWO_LOOPS: &str = "use gpu::{Rgba8, Tile};

pub struct P {
    pub n: int,
}

fn first(n: int) -> int {
    let mut found = 0;
    let mut i = 1;
    'rows: while i < n {
        let mut j = 1;
        while j < n {
            if i * j == 6 {
                found = i;
                break 'rows;
            }
            j += 1;
        }
        i += 1;
    }
    found
}

#[kernel]
pub fn k(p: &P, out: &mut Tile) {
    let v = if first(p.n) > 1 { 1.0 } else { 0.0 };
    out.each(|_x, _y| Rgba8::grey(v));
}
";

#[test]
fn a_shape_wgsl_cannot_structure_keeps_the_dispatcher_and_validates() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gpu-two-loops");
    std::fs::create_dir_all(dir.join("src")).expect("a directory");
    let mods = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/gpu");
    let mods = std::fs::canonicalize(mods).expect("mods/gpu");
    let mods = mods.to_string_lossy().replace('\\', "/");
    let mods = mods.strip_prefix("//?/").unwrap_or(&mods);
    std::fs::write(
        dir.join("moddil.toml"),
        format!(
            "[mod]
name = \"twoloops\"
edition = \"2027\"

[dependencies]
gpu = {{ path = \"{mods}\" }}
"
        ),
    )
    .expect("written");
    std::fs::write(dir.join("src/lib.moddil"), TWO_LOOPS).expect("written");
    let wgsl = wgsl(&dir, "k");
    // `first` keeps the loop-over-switch; the kernel and its closure are structured.
    assert!(wgsl.contains("switch"), "{wgsl}");
    assert!(wgsl.contains("@compute @workgroup_size(8, 8, 1)"), "{wgsl}");
}

#[test]
fn a_kernel_with_arrays_and_slices_lowers_to_valid_wgsl() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/gpu/tests");
    let wgsl = wgsl(&root, "bands");
    // Its own array, the parameters' array, and the slices of it a helper is given.
    assert!(
        wgsl.contains("array<i32, 4>") && wgsl.contains("array<f32, 4>"),
        "{wgsl}"
    );
    assert!(wgsl.contains("struct Slice"), "{wgsl}");
}
