use crate::opcodes::Instruction;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::ptr::NonNull;

pub const QFR_MAGIC_V1: &[u8; 4] = b"QFR1";
pub const QFR_MAGIC_V2: &[u8; 4] = b"QFR!";
pub const QFR_VERSION_V1: u32 = 1;
pub const QFR_VERSION_V2: u16 = 2;

/// Legacy entry point (compiler side)
#[derive(Debug, Clone)]
pub struct EntryPoint {
    pub name: String,
    pub code_offset: u32,
}

/// Legacy const pool entry (compiler side)
#[derive(Debug, Clone)]
pub enum ConstEntry {
    I64(i64),
    F64(f64),
    String(String),
}

/// Legacy program representation used by the compiler
#[derive(Debug, Clone)]
pub struct QfrProgram {
    pub entries: Vec<EntryPoint>,
    pub const_pool: Vec<ConstEntry>,
    pub code: Vec<Instruction>,
    pub const_map: HashMap<String, u32>,
    pub ema_alphas: Vec<f64>,
}

impl QfrProgram {
    pub fn new() -> Self {
        QfrProgram {
            entries: Vec::new(),
            const_pool: Vec::new(),
            code: Vec::new(),
            const_map: HashMap::new(),
            ema_alphas: Vec::new(),
        }
    }

    pub fn entry_offset(&self, name: &str) -> Option<u32> {
        self.entries.iter().find(|e| e.name == name).map(|e| e.code_offset)
    }

    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.const_map.get(s) {
            return idx;
        }
        let idx = self.const_pool.len() as u32;
        self.const_pool.push(ConstEntry::String(s.to_string()));
        self.const_map.insert(s.to_string(), idx);
        idx
    }

    pub fn intern_i64(&mut self, v: i64) -> u32 {
        for (i, entry) in self.const_pool.iter().enumerate() {
            if let ConstEntry::I64(val) = entry {
                if *val == v {
                    return i as u32;
                }
            }
        }
        let idx = self.const_pool.len() as u32;
        self.const_pool.push(ConstEntry::I64(v));
        idx
    }

    pub fn intern_f64(&mut self, v: f64) -> u32 {
        for (i, entry) in self.const_pool.iter().enumerate() {
            if let ConstEntry::F64(val) = entry {
                if val.to_bits() == v.to_bits() {
                    return i as u32;
                }
            }
        }
        let idx = self.const_pool.len() as u32;
        self.const_pool.push(ConstEntry::F64(v));
        idx
    }

    pub fn save(&self, path: &str) -> Result<(), String> {
        let bytes = serialize_v1(self);
        std::fs::write(path, bytes).map_err(|e| format!("write {}: {}", path, e))
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path, e))?;
        if data.len() >= 4 && &data[0..4] == QFR_MAGIC_V2 {
            deserialize_binarized(&data)
        } else {
            deserialize_v1(&data)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Zero-Copy Mmap Architecture (QFRv2)
// ═══════════════════════════════════════════════════════════════════════════════

/// Binary header — byte-exact layout for memory mapping.
/// Total header size: 64 bytes (cache-line aligned).
#[repr(C, align(64))]
pub struct QfrBinarized {
    pub magic: [u8; 4],         // "QFR!"
    pub version: u16,           // 2
    pub entry_count: u16,       // number of entry points
    pub num_constants: u32,     // number of f64 constants
    pub num_instructions: u32,  // number of u64 instructions
    pub persist_mask: [u64; 4], // 256-bit hot-reload mask
    _reserved: [u8; 16],        // pad to 64 bytes
}

/// Entry point descriptor in the binary format.
#[repr(C)]
pub struct QfrEntry {
    pub name_offset: u32,       // byte offset into string data
    pub name_len: u32,          // byte length (not including null)
    pub code_offset: u32,       // instruction offset from code start
    _pad: u32,                  // 16 bytes total
}

/// Zero-copy loader — memory-maps a .qfr file and exposes raw pointers.
pub struct Loader {
    _mmap: memmap2::Mmap,
    pub header: NonNull<QfrBinarized>,
    pub constants_ptr: *const f64,
    pub instructions_ptr: *const u64,
    pub entry_count: u16,
    pub const_count: u32,
    pub instr_count: u32,
}

impl Loader {
    /// Memory-map a .qfr file and validate the header.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let file = File::open(path.as_ref()).map_err(|e| format!("open: {}", e))?;
        let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| format!("mmap: {}", e))?;

        if mmap.len() < 64 {
            return Err("file too small for header".into());
        }

        let ptr = mmap.as_ptr() as *const QfrBinarized;
        let header = unsafe { &*ptr };

        if header.magic != [0x51, 0x46, 0x52, 0x21] {
            return Err("bad magic".into());
        }
        if header.version != 2 {
            return Err(format!("unsupported version: {}", header.version));
        }

        // Pointer to constants: right after header + entry section + string data
        // Entry section: entry_count * 16 bytes
        let entry_bytes = (header.entry_count as u32) * 16u32;
        // String data follows entries, aligned to 8
        let str_data_start = 64u32 + entry_bytes;
        // String data length unknown without scanning, but we need to find end
        // For simplicity, we scan for null terminators
        let mut str_end = str_data_start as usize;
        let mmap_slice = &mmap;
        while str_end < mmap_slice.len() && mmap_slice[str_end] != 0 {
            str_end += 1;
        }
        // Actually string data consists of null-terminated strings: name1\0name2\0...
        // We find end by scanning until str_end reaches the null that ends the last name.
        // Since each name is 0-terminated, the last null marks the end.
        // But we need to know how many strings to scan for: entry_count strings.
        let mut scan_pos = str_data_start as usize;
        for _ in 0..header.entry_count {
            if scan_pos >= mmap_slice.len() {
                return Err("truncated string data".into());
            }
            // skip to null terminator
            while scan_pos < mmap_slice.len() && mmap_slice[scan_pos] != 0 {
                scan_pos += 1;
            }
            // skip the null
            scan_pos += 1;
        }
        str_end = scan_pos;
        // Align to 8 bytes after strings
        let mut const_start = str_end;
        while const_start % 8 != 0 {
            const_start += 1;
        }

        let const_ptr = unsafe { mmap.as_ptr().add(const_start) as *const f64 };
        let code_start = const_start + (header.num_constants as usize) * 8;
        // Align code to 8 (should already be aligned)
        let code_start_aligned = (code_start + 7) & !7;
        let code_ptr = unsafe { mmap.as_ptr().add(code_start_aligned) as *const u64 };

        // Verify code section fits
        let expected_end = code_start_aligned + (header.num_instructions as usize) * 8;
        if expected_end > mmap.len() {
            return Err("truncated code section".into());
        }

        Ok(Loader {
            _mmap: mmap,
            header: NonNull::new(ptr as *mut _).unwrap(),
            constants_ptr: const_ptr,
            instructions_ptr: code_ptr,
            entry_count: header.entry_count,
            const_count: header.num_constants,
            instr_count: header.num_instructions,
        })
    }

    /// Get an entry point's code offset by name (linear scan, called once at init).
    pub fn lookup_entry(&self, name: &str) -> Option<u32> {
        let header = unsafe { self.header.as_ref() };
        if header.entry_count == 0 {
            return None;
        }
        // Entries start at byte 64
        let mmap_slice = &self._mmap;
        for i in 0..header.entry_count as usize {
            let entry_offset = 64 + i * 16;
            if entry_offset + 16 > mmap_slice.len() {
                return None;
            }
            let entry = unsafe {
                &*(mmap_slice.as_ptr().add(entry_offset) as *const QfrEntry)
            };
            if entry.name_len as usize > mmap_slice.len().saturating_sub(entry.name_offset as usize) {
                continue;
            }
            let name_bytes = unsafe {
                std::slice::from_raw_parts(
                    mmap_slice.as_ptr().add(entry.name_offset as usize),
                    entry.name_len as usize,
                )
            };
            if name_bytes == name.as_bytes() {
                return Some(entry.code_offset);
            }
        }
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// V2 Serialization (Binarized format)
// ═══════════════════════════════════════════════════════════════════════════════

/// Serialize a QfrProgram into the zero-copy mmap-compatible binary format.
pub fn serialize_binarized(prog: &QfrProgram) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header
    let persist_mask = [0u64; 4];
    // (persist_mask populated by compiler if needed)
    let entry_count = prog.entries.len() as u16;
    let num_constants = prog.const_pool.iter().filter(|e| matches!(e, ConstEntry::F64(_))).count() as u32;
    let num_instructions = prog.code.len() as u32;

    let reserved = [0u8; 16];
    buf.extend_from_slice(b"QFR!");
    buf.extend_from_slice(&2u16.to_le_bytes());     // version
    buf.extend_from_slice(&entry_count.to_le_bytes());
    buf.extend_from_slice(&num_constants.to_le_bytes());
    buf.extend_from_slice(&num_instructions.to_le_bytes());
    buf.extend_from_slice(unsafe { std::slice::from_raw_parts(persist_mask.as_ptr() as *const u8, 32) });
    buf.extend_from_slice(&reserved);                // 16 bytes
    // Total header: 4+2+2+4+4+32+16 = 64 bytes.

    // Entry section: entry_count * 16 bytes
    // First, collect string data
    let mut string_data = Vec::new();
    for entry in &prog.entries {
        let bytes = entry.name.as_bytes();
        let offset = 64 + (prog.entries.len() * 16) + string_data.len();
        // Write entry descriptor
        buf.extend_from_slice(&(offset as u32).to_le_bytes());  // name_offset
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes()); // name_len
        buf.extend_from_slice(&entry.code_offset.to_le_bytes()); // code_offset
        buf.extend_from_slice(&0u32.to_le_bytes());              // pad
        // Collect name bytes
        string_data.extend_from_slice(bytes);
        string_data.push(0); // null terminator
    }

    // Append string data
    buf.extend_from_slice(&string_data);

    // Align to 8 bytes
    while buf.len() % 8 != 0 {
        buf.push(0);
    }

    // Write f64 constants (from const_pool, f64 entries only)
    for entry in &prog.const_pool {
        if let ConstEntry::F64(v) = entry {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }

    // Align code section to 8 bytes
    while buf.len() % 8 != 0 {
        buf.push(0);
    }

    // Write code section (u64 raw instructions)
    for instr in &prog.code {
        buf.extend_from_slice(&instr.raw().to_le_bytes());
    }

    buf
}

/// Deserialize from binarized format back to QfrProgram (for backward compat).
pub fn deserialize_binarized(data: &[u8]) -> Result<QfrProgram, String> {
    if data.len() < 64 {
        return Err("truncated header".into());
    }
    let magic = &data[0..4];
    if magic != QFR_MAGIC_V2 {
        return Err("bad magic".into());
    }
    let version = u16::from_le_bytes(
        data[4..6].try_into().map_err(|_| "truncated version".to_string())?
    );
    if version != 2 {
        return Err(format!("unsupported version: {}", version));
    }
    let entry_count = u16::from_le_bytes(
        data[6..8].try_into().map_err(|_| "truncated entry count".to_string())?
    ) as usize;
    let num_constants = u32::from_le_bytes(
        data[8..12].try_into().map_err(|_| "truncated const count".to_string())?
    ) as usize;
    let _num_instructions = u32::from_le_bytes(
        data[12..16].try_into().map_err(|_| "truncated instr count".to_string())?
    ) as usize;

    // Read entries
    let mut entries = Vec::with_capacity(entry_count);
    let mut str_data_start = usize::MAX;
    for i in 0..entry_count {
        let off = 64 + i * 16;
        if off + 12 > data.len() {
            return Err("truncated entry section".into());
        }
        let name_off = u32::from_le_bytes(
            data[off..off+4].try_into().map_err(|_| "truncated name_off".to_string())?
        ) as usize;
        let name_len = u32::from_le_bytes(
            data[off+4..off+8].try_into().map_err(|_| "truncated name_len".to_string())?
        ) as usize;
        let code_off = u32::from_le_bytes(
            data[off+8..off+12].try_into().map_err(|_| "truncated code_off".to_string())?
        );
        if str_data_start > name_off { str_data_start = name_off; }
        let name = String::from_utf8(data[name_off..name_off+name_len].to_vec())
            .map_err(|e| format!("utf8 entry: {}", e))?;
        entries.push(EntryPoint { name, code_offset: code_off });
    }

    // String data ends at entry section end
    let _str_end = 64 + entry_count * 16;
    // skip null terminators in string data
    let mut const_start = str_data_start;
    while const_start < data.len() && data[const_start] != 0 {
        const_start += 1;
    }
    const_start += 1; // skip null
    // scan remaining strings
    let mut string_count = 1;
    while string_count < entry_count && const_start < data.len() {
        while const_start < data.len() && data[const_start] != 0 {
            const_start += 1;
        }
        const_start += 1;
        string_count += 1;
    }
    // Align to 8
    while const_start % 8 != 0 {
        const_start += 1;
    }

    // Read f64 constants
    let mut const_pool: Vec<ConstEntry> = Vec::with_capacity(num_constants);
    for i in 0..num_constants {
        let off = const_start + i * 8;
        if off + 8 > data.len() { break; }
        let val = f64::from_le_bytes(
            data[off..off+8].try_into().map_err(|_| "truncated const section".to_string())?
        );
        const_pool.push(ConstEntry::F64(val));
    }
    let const_map = HashMap::new();

    // Read code (after constants)
    let code_start = const_start + num_constants * 8;
    // Need to find actual instruction count from code section
    let code_bytes = data.len().saturating_sub(code_start);
    let code_count = code_bytes / 8;
    let mut code = Vec::with_capacity(code_count);
    for i in 0..code_count {
        let instr_off = code_start + i * 8;
        if instr_off + 8 > data.len() { break; }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&data[instr_off..instr_off+8]);
        code.push(Instruction::decode(&bytes));
    }

    Ok(QfrProgram { entries, const_pool, code, const_map, ema_alphas: Vec::new() })
}

// ═══════════════════════════════════════════════════════════════════════════════
// V1 Serialization (Legacy, backward compat)
// ═══════════════════════════════════════════════════════════════════════════════

pub fn serialize_v1(prog: &QfrProgram) -> Vec<u8> {
    let mut buf = Vec::new();

    buf.extend_from_slice(QFR_MAGIC_V1);
    buf.extend_from_slice(&QFR_VERSION_V1.to_le_bytes());
    buf.extend_from_slice(&(prog.entries.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(prog.const_pool.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(prog.code.len() as u32).to_le_bytes());
    buf.extend_from_slice(&[0u8; 12]);

    let name_base = 32 + (prog.entries.len() as u32 * 8);
    let mut name_data = Vec::new();
    let mut name_offsets: Vec<u32> = Vec::new();
    for entry in &prog.entries {
        let off = name_base + name_data.len() as u32;
        name_offsets.push(off);
        name_data.extend_from_slice(entry.name.as_bytes());
        name_data.push(0);
    }
    for (i, entry) in prog.entries.iter().enumerate() {
        buf.extend_from_slice(&name_offsets[i].to_le_bytes());
        buf.extend_from_slice(&entry.code_offset.to_le_bytes());
    }
    buf.extend_from_slice(&name_data);

    while buf.len() % 8 != 0 {
        buf.push(0);
    }

    for entry in &prog.const_pool {
        match entry {
            ConstEntry::I64(v) => {
                buf.push(0);
                buf.extend_from_slice(&v.to_le_bytes());
                for _ in 0..7 { buf.push(0); }
            }
            ConstEntry::F64(v) => {
                buf.push(1);
                buf.extend_from_slice(&v.to_le_bytes());
                for _ in 0..7 { buf.push(0); }
            }
            ConstEntry::String(s) => {
                buf.push(2);
                let bytes = s.as_bytes();
                let len = bytes.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.extend_from_slice(bytes);
                let total = 5 + bytes.len();
                let pad = (8 - total % 8) % 8;
                for _ in 0..pad { buf.push(0); }
            }
        }
    }

    for instr in &prog.code {
        buf.extend_from_slice(&instr.encode());
    }

    buf
}

pub fn deserialize_v1(data: &[u8]) -> Result<QfrProgram, String> {
    if data.len() < 32 {
        return Err("truncated header".into());
    }
    let magic = &data[0..4];
    if magic != QFR_MAGIC_V1 {
        return Err(format!("bad magic: {:?}", magic));
    }
    let version = u32::from_le_bytes(
        data[4..8].try_into().map_err(|_| "truncated version".to_string())?
    );
    if version != QFR_VERSION_V1 {
        return Err(format!("unsupported version: {}", version));
    }

    let entry_count = u32::from_le_bytes(
        data[8..12].try_into().map_err(|_| "truncated entry count".to_string())?
    ) as usize;
    let const_pool_count = u32::from_le_bytes(
        data[12..16].try_into().map_err(|_| "truncated const count".to_string())?
    ) as usize;
    let code_count = u32::from_le_bytes(
        data[16..20].try_into().map_err(|_| "truncated code count".to_string())?
    ) as usize;

    let mut offset: usize = 32;
    let mut entries = Vec::with_capacity(entry_count);
    let mut name_offs = Vec::new();
    for _ in 0..entry_count {
        if offset + 8 > data.len() {
            return Err("truncated entry points".into());
        }
        let no = u32::from_le_bytes(
            data[offset..offset + 4].try_into().map_err(|_| "truncated name_off".to_string())?
        ) as usize;
        let co = u32::from_le_bytes(
            data[offset + 4..offset + 8].try_into().map_err(|_| "truncated code_off".to_string())?
        );
        name_offs.push(no);
        entries.push(EntryPoint { name: String::new(), code_offset: co });
        offset += 8;
    }

    for (i, no) in name_offs.iter().enumerate() {
        let mut end = *no;
        while end < data.len() && data[end] != 0 { end += 1; }
        if end >= data.len() { return Err("truncated entry name".into()); }
        entries[i].name = String::from_utf8(data[*no..end].to_vec()).map_err(|e| format!("utf8: {}", e))?;
    }

    let max_name_end = name_offs.iter().map(|&n| n as usize).max().unwrap_or(0);
    let mut string_end = max_name_end;
    while string_end < data.len() && data[string_end] != 0 { string_end += 1; }
    offset = string_end + 1;
    while offset % 8 != 0 { offset += 1; }

    let mut const_pool = Vec::with_capacity(const_pool_count);
    for _ in 0..const_pool_count {
        if offset >= data.len() { return Err("truncated const pool".into()); }
        let tag = data[offset];
        offset += 1;
        match tag {
            0 => {
                if offset + 8 > data.len() { return Err("truncated i64 const".into()); }
                let v = i64::from_le_bytes(
                    data[offset..offset + 8].try_into().map_err(|_| "truncated i64".to_string())?
                );
                const_pool.push(ConstEntry::I64(v));
                offset += 15;
            }
            1 => {
                if offset + 8 > data.len() { return Err("truncated f64 const".into()); }
                let v = f64::from_le_bytes(
                    data[offset..offset + 8].try_into().map_err(|_| "truncated f64".to_string())?
                );
                const_pool.push(ConstEntry::F64(v));
                offset += 15;
            }
            2 => {
                if offset + 4 > data.len() { return Err("truncated string const len".into()); }
                let len = u32::from_le_bytes(
                    data[offset..offset + 4].try_into().map_err(|_| "truncated str len".to_string())?
                ) as usize;
                offset += 4;
                if offset + len > data.len() { return Err("truncated string const data".into()); }
                let s = String::from_utf8(data[offset..offset + len].to_vec())
                    .map_err(|e| format!("utf8: {}", e))?;
                const_pool.push(ConstEntry::String(s));
                offset += len;
                while offset % 8 != 0 { offset += 1; }
            }
            _ => return Err(format!("bad const pool tag: {}", tag)),
        }
    }

    if offset + code_count * 8 > data.len() {
        return Err("truncated code section".into());
    }
    let mut code = Vec::with_capacity(code_count);
    for _ in 0..code_count {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&data[offset..offset + 8]);
        code.push(Instruction::decode(&bytes));
        offset += 8;
    }

    let mut const_map = HashMap::new();
    for (i, entry) in const_pool.iter().enumerate() {
        if let ConstEntry::String(s) = entry {
            const_map.insert(s.clone(), i as u32);
        }
    }

    Ok(QfrProgram { entries, const_pool, code, const_map, ema_alphas: Vec::new() })
}

// Alias for backward compatibility
pub fn serialize(prog: &QfrProgram) -> Vec<u8> { serialize_v1(prog) }
pub fn deserialize(data: &[u8]) -> Result<QfrProgram, String> { deserialize_v1(data) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::Opcode;

    // ── V1 roundtrip tests (keep original) ──

    #[test]
    fn serialize_deserialize_roundtrip_preserves_entries_consts_and_code() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "on_trade".into(), code_offset: 0 });
        let s_idx = prog.intern_string("btcusdt");
        let _ = prog.intern_i64(42);
        let _ = prog.intern_f64(3.14);
        prog.code.push(Instruction::rrr(Opcode::Add, 1, 2, 3));
        prog.code.push(Instruction::single(Opcode::Ret));

        let bytes = serialize_v1(&prog);
        assert!(bytes.len() > 32);
        let parsed = deserialize_v1(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "on_trade");
        assert_eq!(parsed.entries[0].code_offset, 0);
        assert_eq!(parsed.const_pool.len(), 3);
        assert_eq!(parsed.code.len(), 2);
        assert_eq!(parsed.code[0].opcode(), Opcode::Add);
        assert_eq!(parsed.const_map.get("btcusdt"), Some(&s_idx));
    }

    #[test]
    fn deserialize_empty_bytes_returns_error() {
        assert!(deserialize_v1(&[]).is_err());
    }

    #[test]
    fn deserialize_wrong_magic_bytes_returns_error() {
        assert!(deserialize_v1(&[0, 0, 0, 0]).is_err());
    }

    #[test]
    fn deserialize_truncated_header_returns_error() {
        assert!(deserialize_v1(b"QFR1").is_err());
    }

    #[test]
    fn serialize_deserialize_ten_entries_all_preserved() {
        let mut prog = QfrProgram::new();
        for i in 0..10 {
            prog.entries.push(EntryPoint { name: format!("fn{}", i), code_offset: i as u32 });
        }
        prog.code.push(Instruction::single(Opcode::Halt));
        let bytes = serialize_v1(&prog);
        let parsed = deserialize_v1(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 10);
        assert_eq!(parsed.entries[5].name, "fn5");
    }

    #[test]
    fn serialize_deserialize_hundred_constants_all_preserved() {
        let mut prog = QfrProgram::new();
        for i in 0..100 { prog.intern_i64(i); }
        prog.code.push(Instruction::single(Opcode::Halt));
        let bytes = serialize_v1(&prog);
        let parsed = deserialize_v1(&bytes).unwrap();
        assert_eq!(parsed.const_pool.len(), 100);
    }

    #[test]
    fn serialize_deserialize_empty_const_pool_succeeds() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code.push(Instruction::single(Opcode::Ret));
        let bytes = serialize_v1(&prog);
        let parsed = deserialize_v1(&bytes).unwrap();
        assert!(parsed.const_pool.is_empty());
        assert_eq!(parsed.code.len(), 1);
    }

    #[test]
    fn save_load_roundtrip_file() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "on_trade".into(), code_offset: 10 });
        let _ = prog.intern_string("btcusdt");
        let _ = prog.intern_i64(42);
        let _ = prog.intern_f64(3.14);
        prog.code.push(Instruction::rrr(Opcode::Add, 1, 2, 3));
        prog.code.push(Instruction::single(Opcode::Ret));

        let path = "test_ir_save_load.qfr";
        prog.save(path).unwrap();
        let loaded = QfrProgram::load(path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].name, "on_trade");
        assert_eq!(loaded.entries[0].code_offset, 10);
        assert_eq!(loaded.const_pool.len(), 3);
        assert_eq!(loaded.code.len(), 2);
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        assert!(QfrProgram::load("nonexistent.qfr").is_err());
    }

    // ── V2 binarized format tests ──

    #[test]
    fn binarized_roundtrip_preserves_entries_and_code() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "on_trade".into(), code_offset: 0 });
        prog.entries.push(EntryPoint { name: "on_eval".into(), code_offset: 10 });
        let _ = prog.intern_f64(3.14);
        prog.code.push(Instruction::rrr(Opcode::Add, 1, 2, 3));
        prog.code.push(Instruction::single(Opcode::Ret));

        let bytes = serialize_binarized(&prog);
        let parsed = deserialize_binarized(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, "on_trade");
        assert_eq!(parsed.entries[1].name, "on_eval");
        assert_eq!(parsed.code.len(), 2);
    }

    #[test]
    fn binarized_invalid_magic_returns_error() {
        assert!(deserialize_binarized(&[0; 64]).is_err());
    }

    #[test]
    fn binarized_loader_missing_file() {
        let result = Loader::load("nonexistent_loader.qfr");
        assert!(result.is_err());
    }

    #[test]
    fn binarized_save_load_with_loader() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        let _ = prog.intern_f64(42.0);
        prog.code.push(Instruction::rri(Opcode::Ldi, 0, 0, 42));
        prog.code.push(Instruction::single(Opcode::Halt));

        let path = "test_binarized_loader.qfr";
        let bytes = serialize_binarized(&prog);
        std::fs::write(path, &bytes).unwrap();

        let loader = Loader::load(path).unwrap();
        unsafe {
            assert_eq!((*loader.header.as_ptr()).magic, [0x51, 0x46, 0x52, 0x21]);
            assert_eq!((*loader.header.as_ptr()).version, 2);
            assert_eq!((*loader.header.as_ptr()).num_instructions, 2);
        }
        let entry = loader.lookup_entry("main");
        assert_eq!(entry, Some(0));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_auto_detects_v1_vs_v2() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "test".into(), code_offset: 0 });
        prog.code.push(Instruction::single(Opcode::Halt));

        // V1 format
        let v1_bytes = serialize_v1(&prog);
        let path_v1 = "test_auto_v1.qfr";
        std::fs::write(path_v1, &v1_bytes).unwrap();
        let loaded_v1 = QfrProgram::load(path_v1).unwrap();
        assert_eq!(loaded_v1.entries.len(), 1);
        let _ = std::fs::remove_file(path_v1);

        // V2 format
        let v2_bytes = serialize_binarized(&prog);
        let path_v2 = "test_auto_v2.qfr";
        std::fs::write(path_v2, &v2_bytes).unwrap();
        let loaded_v2 = QfrProgram::load(path_v2).unwrap();
        assert_eq!(loaded_v2.entries.len(), 1);
        let _ = std::fs::remove_file(path_v2);
    }
}
