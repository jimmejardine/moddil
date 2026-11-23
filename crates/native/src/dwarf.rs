//! Line tables for a release build's object (`moddil build --release`): DWARF that says, for
//! each function compiled from source, where it starts and ends, and which file, line and column
//! each run of its machine code comes from, so debuggers and profilers can show the source.
//!
//! The lowering sets a source location on the code of each MIR statement and terminator: an
//! index into the run's table of places (a file and an offset in it). Once Cranelift has compiled
//! a function, its code's runs and their locations are kept ([`FnLines`]). With the object
//! written, this writes `.debug_abbrev`, `.debug_info` (a compile unit, and a subprogram for each
//! function), `.debug_line`, `.debug_ranges` and `.debug_str`, their addresses relocated against
//! the functions' symbols. An ELF object's sections name each other by relocated offsets; a
//! Mach-O object's by plain ones, as `dsymutil` reads them. COFF gets none (debuggers there read
//! PDB).

use cranelift_module::FuncId;
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Range, RangeList,
    Sections, Writer,
};
use gimli::{Encoding, Format, LineEncoding, RunTimeEndian, SectionId};
use object::write::{Object, Relocation, SymbolId};
use object::{BinaryFormat, RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};

/// A compiled function's lines: its name, its code's size, and each run of its code (from its
/// offset) with the place it comes from (an index into the places, or `None`).
pub(crate) struct FnLines {
    pub(crate) id: FuncId,
    pub(crate) name: String,
    pub(crate) size: u32,
    pub(crate) runs: Vec<(u32, Option<u32>)>,
}

/// What a relocation in a debug section points to.
#[derive(Clone, Copy)]
enum Target {
    /// A function, by its index in the functions written.
    Symbol(usize),
    /// Another debug section.
    Section(SectionId),
}

/// A debug section being written, and the relocations it needs.
#[derive(Clone)]
struct Section {
    data: EndianVec<RunTimeEndian>,
    relocs: Vec<(u64, u8, Target, i64)>,
}

impl Writer for Section {
    type Endian = RunTimeEndian;

    fn endian(&self) -> RunTimeEndian {
        self.data.endian()
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn write(&mut self, bytes: &[u8]) -> gimli::write::Result<()> {
        self.data.write(bytes)
    }

    fn write_at(&mut self, offset: usize, bytes: &[u8]) -> gimli::write::Result<()> {
        self.data.write_at(offset, bytes)
    }

    fn write_address(&mut self, address: Address, size: u8) -> gimli::write::Result<()> {
        match address {
            Address::Constant(v) => self.write_udata(v, size),
            Address::Symbol { symbol, addend } => {
                let at = self.len() as u64;
                self.relocs.push((at, size, Target::Symbol(symbol), addend));
                self.write_udata(0, size)
            }
        }
    }

    fn write_offset(
        &mut self,
        val: usize,
        section: SectionId,
        size: u8,
    ) -> gimli::write::Result<()> {
        let at = self.len() as u64;
        self.relocs
            .push((at, size, Target::Section(section), val as i64));
        self.write_udata(val as u64, size)
    }

    fn write_offset_at(
        &mut self,
        offset: usize,
        val: usize,
        section: SectionId,
        size: u8,
    ) -> gimli::write::Result<()> {
        self.relocs
            .push((offset as u64, size, Target::Section(section), val as i64));
        self.write_udata_at(offset, val as u64, size)
    }
}

/// Writes the line tables of `fns` into `obj`: `symbol` gives each function's symbol, `places`
/// the file and offset of each place, and `files` each file's path and text.
pub(crate) fn write(
    obj: &mut Object<'_>,
    fns: &[FnLines],
    symbol: impl Fn(FuncId) -> SymbolId,
    places: &[(u32, u32)],
    files: &[(String, String)],
) -> Result<(), String> {
    let format = obj.format();
    if !matches!(format, BinaryFormat::Elf | BinaryFormat::MachO) || fns.is_empty() {
        return Ok(());
    }
    let e = |e: gimli::write::Error| format!("writing line tables: {e}");
    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let mut dwarf = DwarfUnit::new(encoding);
    let first = files.first().map_or("main.moddil", |(p, _)| p.as_str());
    let cwd = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    dwarf.unit.line_program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(cwd.clone().into_bytes()),
        None,
        LineString::String(first.as_bytes().to_vec()),
        None,
    );
    let dir = dwarf.unit.line_program.default_directory();
    let file_ids: Vec<_> = files
        .iter()
        .map(|(path, _)| {
            let name = if path.is_empty() { "<source>" } else { path };
            dwarf.unit.line_program.add_file(
                LineString::String(name.as_bytes().to_vec()),
                dir,
                None,
            )
        })
        .collect();
    let line_of = |place: u32| -> Option<(usize, u64, u64)> {
        let (file, offset) = *places.get(place as usize)?;
        let (_, text) = files.get(file as usize)?;
        let (line, col) = moddil_syntax::line_col(text, offset);
        Some((file as usize, u64::from(line), u64::from(col)))
    };
    // The compile unit, and its functions.
    let root = dwarf.unit.root();
    let producer = dwarf
        .strings
        .add(format!("moddil {}", env!("CARGO_PKG_VERSION")));
    let name = dwarf.strings.add(first);
    let comp_dir = dwarf.strings.add(cwd);
    {
        let cu = dwarf.unit.get_mut(root);
        cu.set(gimli::DW_AT_producer, AttributeValue::StringRef(producer));
        cu.set(gimli::DW_AT_name, AttributeValue::StringRef(name));
        cu.set(gimli::DW_AT_comp_dir, AttributeValue::StringRef(comp_dir));
    }
    let mut ranges = Vec::new();
    for (k, f) in fns.iter().enumerate() {
        let start = Address::Symbol {
            symbol: k,
            addend: 0,
        };
        ranges.push(Range::StartLength {
            begin: start,
            length: u64::from(f.size),
        });
        let program = &mut dwarf.unit.line_program;
        program.begin_sequence(Some(start));
        let mut decl = None;
        for &(offset, place) in &f.runs {
            let Some((file, line, col)) = place.and_then(line_of) else {
                continue;
            };
            decl.get_or_insert((file, line));
            let row = program.row();
            row.address_offset = u64::from(offset);
            row.file = file_ids[file];
            row.line = line;
            row.column = col;
            program.generate_row();
        }
        program.end_sequence(u64::from(f.size));
        let sub = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        let name = dwarf.strings.add(f.name.as_str());
        let die = dwarf.unit.get_mut(sub);
        die.set(gimli::DW_AT_name, AttributeValue::StringRef(name));
        die.set(gimli::DW_AT_low_pc, AttributeValue::Address(start));
        die.set(
            gimli::DW_AT_high_pc,
            AttributeValue::Udata(u64::from(f.size)),
        );
        if let Some((file, line)) = decl {
            die.set(
                gimli::DW_AT_decl_file,
                AttributeValue::FileIndex(Some(file_ids[file])),
            );
            die.set(gimli::DW_AT_decl_line, AttributeValue::Udata(line));
        }
    }
    let list = dwarf.unit.ranges.add(RangeList(ranges));
    let cu = dwarf.unit.get_mut(root);
    cu.set(
        gimli::DW_AT_low_pc,
        AttributeValue::Address(Address::Constant(0)),
    );
    cu.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(list));

    // The hosts Cranelift builds for here (x86-64, AArch64) are little-endian.
    let endian = RunTimeEndian::Little;
    let mut sections = Sections::new(Section {
        data: EndianVec::new(endian),
        relocs: Vec::new(),
    });
    dwarf.write(&mut sections).map_err(e)?;
    // Each section into the object, then their relocations (a section can name one written
    // after it).
    let segment = obj
        .segment_name(object::write::StandardSegment::Debug)
        .to_vec();
    let mut ids = std::collections::HashMap::new();
    let mut written = Vec::new();
    sections.for_each(|id, s| -> Result<(), String> {
        if s.data.len() == 0 {
            return Ok(());
        }
        let name = match format {
            BinaryFormat::MachO => id.name().replacen('.', "__", 1),
            _ => id.name().to_string(),
        };
        let sec = obj.add_section(segment.clone(), name.into_bytes(), SectionKind::Debug);
        obj.append_section_data(sec, s.data.slice(), 1);
        ids.insert(id, sec);
        written.push((sec, s.relocs.clone()));
        Ok(())
    })?;
    let symbols: Vec<SymbolId> = fns.iter().map(|f| symbol(f.id)).collect();
    for (sec, relocs) in written {
        for (offset, size, target, addend) in relocs {
            let symbol = match target {
                Target::Symbol(k) => symbols[k],
                // A Mach-O object's debug sections name each other by plain offsets.
                Target::Section(_) if format == BinaryFormat::MachO => continue,
                Target::Section(s) => match ids.get(&s) {
                    Some(t) => obj.section_symbol(*t),
                    None => continue,
                },
            };
            obj.add_relocation(
                sec,
                Relocation {
                    offset,
                    symbol,
                    addend,
                    flags: RelocationFlags::Generic {
                        kind: RelocationKind::Absolute,
                        encoding: RelocationEncoding::Generic,
                        size: size * 8,
                    },
                },
            )
            .map_err(|e| format!("writing line tables: {e}"))?;
        }
    }
    Ok(())
}
