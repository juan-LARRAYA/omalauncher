//! Flat little-endian sections, memory-mapped: the files `exportar.py` writes.

use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;

pub struct BinFile {
    map: Mmap,
    sections: HashMap<String, (usize, usize)>,
}

pub struct Texts<'a> {
    off: &'a [u32],
    txt: &'a [u8],
}

impl Texts<'_> {
    pub fn get(&self, i: usize) -> &str {
        std::str::from_utf8(&self.txt[self.off[i] as usize..self.off[i + 1] as usize]).unwrap_or("")
    }
}

impl BinFile {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
        // Safety: the file is written aside and renamed into place, never changed while mapped.
        let map = unsafe { Mmap::map(&file) }.map_err(|e| format!("{path}: {e}"))?;
        if map.len() < 16 || &map[..8] != b"MAPASBIN" {
            return Err(format!("{path}: not a MAPASBIN file"));
        }
        let word = |at: usize| u64::from_le_bytes(map[at..at + 8].try_into().unwrap()) as usize;
        let count = word(8);
        let mut sections = HashMap::new();
        for i in 0..count {
            let at = 16 + 40 * i;
            let name = &map[at..at + 24];
            let end = name.iter().position(|b| *b == 0).unwrap_or(24);
            sections.insert(String::from_utf8_lossy(&name[..end]).into_owned(), (word(at + 24), word(at + 32)));
        }
        Ok(Self { map, sections })
    }

    pub fn bytes(&self, name: &str) -> &[u8] {
        let (at, len) = self.sections[name];
        &self.map[at..at + len]
    }

    pub fn slice<T: bytemuck::Pod>(&self, name: &str) -> &[T] {
        bytemuck::cast_slice(self.bytes(name))
    }

    pub fn texts(&self, name: &str) -> Texts<'_> {
        Texts { off: self.slice(&format!("{name}_off")), txt: self.bytes(&format!("{name}_txt")) }
    }
}
