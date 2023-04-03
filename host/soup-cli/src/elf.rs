use object::{
    elf::{FileHeader32, PT_LOAD},
    read::elf::{FileHeader, ProgramHeader},
    LittleEndian, Object, ObjectSection,
};
use std::{
    cmp::Ordering,
    error::Error,
    fs,
    ops::Range,
};

pub struct Loadable {
    pub addr: u32,
    pub data: Vec<u8>,
}

pub fn parse_loadable(s: String) -> Result<Loadable, Box<dyn Error>> {
    let bin_data = fs::read(&s)?;
    let obj_file = object::File::parse(&*bin_data)?;

    match obj_file.format() {
        object::BinaryFormat::Elf => {}
        _ => return Err("Unsupported format. Only elf.".into()),
    }

    if !obj_file.is_little_endian() {
        return Err("Only LE supported".into());
    }

    let file_kind = object::FileKind::parse(&*bin_data)?;

    match file_kind {
        object::FileKind::Elf32 => {}
        fk => return Err(format!("Unsupported file type: {:?}", fk).into()),
    }

    let elf_header = FileHeader32::<LittleEndian>::parse(&*bin_data)?;
    let endian = elf_header.endian()?;

    let mut lowest_addr = u64::MAX;
    let mut highest_addr = u64::MIN;
    let mut bin_contents = vec![];

    // NOTE: Using https://github.com/probe-rs/probe-rs/blob/5a29e83847118c3999a2ca0ab017f080719b8ae5/probe-rs/src/flashing/download.rs#L194
    // as a reference
    for segment in elf_header.program_headers(endian, &*bin_data)? {
        let p_paddr: u64 = segment.p_paddr(endian).into();
        let _p_vaddr: u64 = segment.p_vaddr(endian).into();
        let _flags = segment.p_flags(endian);

        let segment_data = segment.data(endian, &*bin_data).map_err(|_| {
            "Failed to get segment data"
        })?;

        let load = segment.p_type(endian) == PT_LOAD;
        let _sz = segment_data.len();

        // println!("{p_paddr:08X}, {p_vaddr:08X}, {flags:08X}, sz: {sz}, l?: {load}");

        if !load {
            continue;
        }

        let (segment_offset, segment_filesize) = segment.file_range(endian);

        let sector: core::ops::Range<u64> = segment_offset..segment_offset + segment_filesize;

        for section in obj_file.sections() {
            let (section_offset, section_filesize) = match section.file_range() {
                Some(range) => range,
                None => continue,
            };

            if sector.contains_range(&(section_offset..section_offset + section_filesize)) {
                // println!("  -> Matching section: {:?}", section.name()?);
                // println!("  -> {:08X}, {}", p_paddr, segment_data.len());

                lowest_addr = lowest_addr.min(p_paddr);
                let fsz: u32 = segment.p_filesz(endian);
                let fsz64: u64 = fsz.into();
                assert_eq!(segment_data.len(), fsz.try_into()?);

                highest_addr = highest_addr.max(p_paddr + fsz64);
                bin_contents.push((p_paddr, segment_data));

                for (offset, relocation) in section.relocations() {
                    return Err(format!(
                        "I can't do relocations sorry: ({}) {:?}, {:?}",
                        section.name()?,
                        offset,
                        relocation,
                    ).into());
                }
            }
        }
    }

    match lowest_addr.cmp(&highest_addr) {
        Ordering::Less => {}
        Ordering::Equal => return Err("Empty file?".into()),
        Ordering::Greater if bin_contents.is_empty() => return Err("No sections found?".into()),
        Ordering::Greater => return Err("Start is after end?".into()),
    }

    let ttl_len: usize = (highest_addr - lowest_addr).try_into()?;
    // println!("start: 0x{lowest_addr:08X}");
    // println!("end:   0x{highest_addr:08X}");
    // println!("size:  {ttl_len}");

    let mut output = vec![0x00u8; ttl_len];
    for (addr, data) in bin_contents.iter() {
        let adj_addr = (addr - lowest_addr).try_into()?;
        let size = data.len();
        output[adj_addr..][..size].copy_from_slice(data);
    }

    Ok(Loadable { addr: lowest_addr.try_into()?, data: output })
}

///////
// https://github.com/probe-rs/probe-rs/blob/ef635f213a2741ebac4c1ccfb700230992dd10a6/probe-rs-target/src/memory.rs#L102-L130
///////

pub trait MemoryRange {
    /// Returns true if `self` contains `range` fully.
    fn contains_range(&self, range: &Range<u64>) -> bool;

    /// Returns true if `self` intersects `range` partially.
    fn intersects_range(&self, range: &Range<u64>) -> bool;
}

impl MemoryRange for Range<u64> {
    fn contains_range(&self, range: &Range<u64>) -> bool {
        if range.end == 0 {
            false
        } else {
            self.contains(&range.start) && self.contains(&(range.end - 1))
        }
    }

    fn intersects_range(&self, range: &Range<u64>) -> bool {
        if range.end == 0 {
            false
        } else {
            self.contains(&range.start) && !self.contains(&(range.end - 1))
                || !self.contains(&range.start) && self.contains(&(range.end - 1))
                || self.contains_range(range)
                || range.contains_range(self)
        }
    }
}
