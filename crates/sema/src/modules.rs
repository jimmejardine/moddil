//! Names across modules (`modules.md`): what a name means in a module, what a path names, and
//! who can see what.
//!
//! A module's names are the items it declares, the names it imports with `use`, and the modules
//! inside it; then the prelude's, which any of those shadow. A path starts at `crate` (the root
//! module of the mod), at a dependency's name (its root module), at `self`, or at a name in scope,
//! and each further segment is a name in the module before it that the naming module can see.
//! Imports are followed when a name is looked up, so they may name each other in any order.

use crate::items::{Def, ItemTree, PRELUDE_MODULE, Vis};

/// How deeply imports may lead to imports before a name is taken not to resolve: a cycle of
/// imports names nothing.
const MAX_DEPTH: u32 = 32;

/// What a name in a module is, as another module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    /// What it stands for.
    Found(Def),
    /// It names something the naming module cannot see.
    Hidden(Def),
    /// There is nothing by that name.
    Missing,
}

/// Why a path does not resolve: the index of the segment that fails, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// The segment names nothing.
    Unresolved(usize),
    /// The segment names something the naming module cannot see.
    Private(usize, Def),
}

impl ItemTree {
    /// Whether an item of module `owner` that `vis` describes can be named from module `from`.
    pub fn visible(&self, owner: u32, vis: Vis, from: u32) -> bool {
        match vis {
            Vis::Pub => true,
            Vis::Mod => self.modules[owner as usize].mod_ == self.modules[from as usize].mod_,
            Vis::Private => owner == from,
        }
    }

    /// The mod module `m` belongs to.
    pub fn mod_of(&self, m: u32) -> u32 {
        self.modules[m as usize].mod_
    }

    /// What `name` is in module `m`, as module `from` names it: an item, an import or a module
    /// inside it. Not the prelude's names.
    pub fn member(&self, m: u32, name: &str, from: u32) -> Member {
        self.member_depth(m, name, from, 0)
    }

    fn member_depth(&self, m: u32, name: &str, from: u32, depth: u32) -> Member {
        let module = &self.modules[m as usize];
        if let Some(e) = module.items.get(name) {
            return if self.visible(m, e.vis, from) {
                Member::Found(e.def)
            } else {
                Member::Hidden(e.def)
            };
        }
        if let Some(i) = module.imports.iter().find(|i| i.name == name) {
            let Some(def) = self.import_target(m, i, depth) else {
                return Member::Missing;
            };
            return if self.visible(m, i.vis, from) {
                Member::Found(def)
            } else {
                Member::Hidden(def)
            };
        }
        if let Some(&c) = module.children.get(name) {
            return Member::Found(Def::Module(c));
        }
        Member::Missing
    }

    /// What import `i` of module `m` names, if it resolves.
    pub fn import_target(&self, m: u32, i: &crate::items::Import, depth: u32) -> Option<Def> {
        if depth > MAX_DEPTH {
            return None;
        }
        let segs: Vec<&str> = i.path.iter().map(|(s, _)| s.as_str()).collect();
        match self.path_depth(m, &segs, false, depth + 1) {
            Ok((def, n)) if n == segs.len() => Some(def),
            Ok((Def::Adt(a, ..), n)) if n + 1 == segs.len() => {
                let v = self.adts[a as usize].variant(segs[n])?;
                Some(Def::Variant(a, v))
            }
            _ => None,
        }
    }

    /// What `name` means in module `from`, unqualified: its own names, then the prelude's (or
    /// the prelude's first, for derived code, which must name the prelude's traits whatever the
    /// module calls its own items).
    pub fn lookup(&self, from: u32, name: &str, prelude_first: bool) -> Option<Def> {
        self.lookup_depth(from, name, prelude_first, 0)
    }

    fn lookup_depth(&self, from: u32, name: &str, prelude_first: bool, depth: u32) -> Option<Def> {
        let own = || match self.member_depth(from, name, from, depth) {
            Member::Found(d) => Some(d),
            _ => None,
        };
        let prelude = || match self.member_depth(PRELUDE_MODULE, name, from, depth) {
            Member::Found(d) => Some(d),
            _ => None,
        };
        if prelude_first {
            prelude().or_else(own)
        } else {
            own().or_else(prelude)
        }
    }

    /// What the leading segments of the path `segs`, named in module `from`, stand for: the
    /// longest prefix that names modules and then one item, and how many segments it takes. The
    /// rest (a variant, an associated function) are the caller's.
    pub fn path(
        &self,
        from: u32,
        segs: &[&str],
        prelude_first: bool,
    ) -> Result<(Def, usize), PathError> {
        self.path_depth(from, segs, prelude_first, 0)
    }

    fn path_depth(
        &self,
        from: u32,
        segs: &[&str],
        prelude_first: bool,
        depth: u32,
    ) -> Result<(Def, usize), PathError> {
        let Some(&first) = segs.first() else {
            return Err(PathError::Unresolved(0));
        };
        let here = self.mod_of(from);
        let mod_def = self.mods.get(here as usize);
        let mut def = match first {
            "crate" => match mod_def {
                Some(m) => Def::Module(m.root),
                None => return Err(PathError::Unresolved(0)),
            },
            "self" => Def::Module(from),
            name => match self.lookup_depth(from, name, prelude_first, depth) {
                Some(d) => d,
                None => match mod_def.and_then(|m| m.deps.get(name)) {
                    Some(&d) => Def::Module(self.mods[d as usize].root),
                    None => return Err(PathError::Unresolved(0)),
                },
            },
        };
        for (i, seg) in segs.iter().enumerate().skip(1) {
            let Def::Module(m) = def else {
                return Ok((def, i));
            };
            def = match self.member_depth(m, seg, from, depth) {
                Member::Found(d) => d,
                Member::Hidden(d) => return Err(PathError::Private(i, d)),
                Member::Missing => return Err(PathError::Unresolved(i)),
            };
        }
        Ok((def, segs.len()))
    }

    /// How module `m` is written as a path: `crate::book::levels`, or the prelude's.
    pub fn module_path(&self, m: u32) -> String {
        let module = &self.modules[m as usize];
        if m == PRELUDE_MODULE {
            return "the prelude".to_string();
        }
        let mut s = "crate".to_string();
        for p in &module.path {
            s.push_str("::");
            s.push_str(p);
        }
        s
    }

    /// How the item `def` is called, for messages.
    pub fn def_name(&self, def: Def) -> String {
        match def {
            Def::Fn(f) => self.fns[f as usize].name.clone(),
            Def::Adt(a) => self.adts[a as usize].name.clone(),
            Def::Trait(t) => self.traits[t as usize].name.clone(),
            Def::Variant(a, v) => self.variant(a, v).name.clone(),
            Def::Module(m) => self.modules[m as usize]
                .path
                .last()
                .cloned()
                .unwrap_or_else(|| "crate".to_string()),
        }
    }

    /// The `fn main` the program runs: an executable's main module's, or else the root module's
    /// of the mod being built.
    pub fn main_fn(&self) -> Option<u32> {
        let entry = self
            .modules
            .iter()
            .position(|m| m.mod_ == 0 && m.exe)
            .or_else(|| self.mods.first().map(|m| m.root as usize))?;
        match self.modules[entry].items.get("main") {
            Some(e) => match e.def {
                Def::Fn(f) => Some(f),
                _ => None,
            },
            None => None,
        }
    }

    /// Whether function `f` is an entry point `main`: a free function `main` of the root module
    /// of the mod being built, or of one of its executables' main modules.
    pub fn is_main(&self, f: u32) -> bool {
        let sig = &self.fns[f as usize];
        if sig.name != "main" || !sig.src.is_file() {
            return false;
        }
        let module = &self.modules[sig.module as usize];
        module.mod_ == 0
            && (module.exe || self.mods.first().is_some_and(|m| m.root == sig.module))
            && module.items.get("main").map(|e| e.def) == Some(Def::Fn(f))
    }
}
