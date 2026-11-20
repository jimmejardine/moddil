//! The reference interpreter must stay independent of the compiler: among workspace crates it may
//! depend only on `moddil-syntax`, directly or transitively (`specs/compiler.md`, "Two
//! interpreters"). General-purpose crates from crates.io are allowed.

use std::collections::{BTreeSet, HashMap, HashSet};

use cargo_metadata::{DependencyKind, MetadataCommand, PackageId};

const ALLOWED: &[&str] = &["moddil-syntax"];

#[test]
fn refinterp_depends_only_on_syntax() {
    let metadata = MetadataCommand::new()
        .manifest_path(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .exec()
        .expect("cargo metadata");
    let resolve = metadata.resolve.as_ref().expect("dependency graph");
    let members: HashSet<&PackageId> = metadata.workspace_members.iter().collect();
    let names: HashMap<&PackageId, &str> = metadata
        .packages
        .iter()
        .map(|p| (&p.id, p.name.as_str()))
        .collect();
    let nodes: HashMap<&PackageId, _> = resolve.nodes.iter().map(|n| (&n.id, n)).collect();

    let root = metadata
        .packages
        .iter()
        .find(|p| p.name.as_str() == "moddil-refinterp")
        .map(|p| &p.id)
        .expect("moddil-refinterp in the workspace");

    // Walk normal and build dependencies (not dev-dependencies, which only tests use).
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut seen: HashSet<&PackageId> = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for dep in &nodes[id].deps {
            let runtime = dep
                .dep_kinds
                .iter()
                .any(|k| matches!(k.kind, DependencyKind::Normal | DependencyKind::Build));
            if !runtime {
                continue;
            }
            if members.contains(&dep.pkg) {
                reached.insert(names[&dep.pkg]);
            }
            stack.push(&dep.pkg);
        }
    }

    let forbidden: Vec<&str> = reached
        .into_iter()
        .filter(|n| !ALLOWED.contains(n))
        .collect();
    assert!(
        forbidden.is_empty(),
        "moddil-refinterp must depend only on {ALLOWED:?} among workspace crates, \
         but reaches {forbidden:?}"
    );
}
