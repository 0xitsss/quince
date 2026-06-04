use crate::opcodes::Instruction;
use std::collections::HashMap;

pub const QFR_MAGIC: &[u8; 4] = b"QFR1";
pub const QFR_VERSION: u32 = 1;

/// Entry point mapping: function name → code offset (in instruction units)
#[derive(Debug, Clone)]
pub struct EntryPoint {
    pub name: String,
    /// Offset in instruction-count from start of code section
    pub code_offset: u32,
}

/// A constant pool entry
#[derive(Debug, Clone)]
pub enum ConstEntry {
    I64(i64),
    F64(f64),
    String(String),
}

/// Parsed .qfr program ready for VM execution
#[derive(Debug, Clone)]
pub struct QfrProgram {
    pub entries: Vec<EntryPoint>,
    pub const_pool: Vec<ConstEntry>,
    pub code: Vec<Instruction>,
    /// name → index in const_pool for fast lookup
    pub const_map: HashMap<String, u32>,
}

impl QfrProgram {
    pub fn new() -> Self {
        QfrProgram {
            entries: Vec::new(),
            const_pool: Vec::new(),
            code: Vec::new(),
            const_map: HashMap::new(),
        }
    }

    /// Find the entry point offset for a function (e.g. "on_trade")
    pub fn entry_offset(&self, name: &str) -> Option<u32> {
        self.entries.iter().find(|e| e.name == name).map(|e| e.code_offset)
    }

    /// Look up a const pool string index, or add it
    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.const_map.get(s) {
            return idx;
        }
        let idx = self.const_pool.len() as u32;
        self.const_pool.push(ConstEntry::String(s.to_string()));
        self.const_map.insert(s.to_string(), idx);
        idx
    }

    /// Look up or add an i64 constant
    pub fn intern_i64(&mut self, v: i64) -> u32 {
        // Linear scan — small pool expected (< 100 entries)
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

    /// Look up or add an f64 constant
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
}

/// Serialize a QfrProgram to .qfr binary format
pub fn serialize(prog: &QfrProgram) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header
    buf.extend_from_slice(QFR_MAGIC);
    buf.extend_from_slice(&QFR_VERSION.to_le_bytes()); // 4 bytes
    buf.extend_from_slice(&(prog.entries.len() as u32).to_le_bytes()); // 4
    buf.extend_from_slice(&(prog.const_pool.len() as u32).to_le_bytes()); // 4
    buf.extend_from_slice(&(prog.code.len() as u32).to_le_bytes()); // 4
    // 12 bytes reserved
    buf.extend_from_slice(&[0u8; 12]);

    // Entry points
    let name_base = 32 + (prog.entries.len() as u32 * 8);
    let mut name_data = Vec::new();
    let mut name_offsets: Vec<u32> = Vec::new();
    for entry in &prog.entries {
        let off = name_base + name_data.len() as u32;
        name_offsets.push(off);
        name_data.extend_from_slice(entry.name.as_bytes());
        name_data.push(0); // null terminator
    }
    for (i, entry) in prog.entries.iter().enumerate() {
        buf.extend_from_slice(&name_offsets[i].to_le_bytes());
        buf.extend_from_slice(&entry.code_offset.to_le_bytes());
    }

    // Name strings
    buf.extend_from_slice(&name_data);

    // Align to 8 bytes
    while buf.len() % 8 != 0 {
        buf.push(0);
    }

    // Const pool (each entry padded to 8 bytes)
    for entry in &prog.const_pool {
        match entry {
            ConstEntry::I64(v) => {
                buf.push(0); // tag
                buf.extend_from_slice(&v.to_le_bytes());
                // tag(1) + data(8) = 9 → pad 7
                for _ in 0..7 { buf.push(0); }
            }
            ConstEntry::F64(v) => {
                buf.push(1); // tag
                buf.extend_from_slice(&v.to_le_bytes());
                // tag(1) + data(8) = 9 → pad 7
                for _ in 0..7 { buf.push(0); }
            }
            ConstEntry::String(s) => {
                buf.push(2); // tag
                let bytes = s.as_bytes();
                let len = bytes.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.extend_from_slice(bytes);
                // tag(1) + len(4) + data(N) = 5+N → align to 8
                let total = 5 + bytes.len();
                let pad = (8 - total % 8) % 8;
                for _ in 0..pad {
                    buf.push(0);
                }
            }
        }
    }

    // Code section
    for instr in &prog.code {
        buf.extend_from_slice(&instr.encode());
    }

    buf
}

/// Deserialize .qfr binary into QfrProgram
pub fn deserialize(data: &[u8]) -> Result<QfrProgram, String> {
    if data.len() < 32 {
        return Err("truncated header".into());
    }

    let magic = &data[0..4];
    if magic != QFR_MAGIC {
        return Err(format!("bad magic: {:?}", magic));
    }

    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != QFR_VERSION {
        return Err(format!("unsupported version: {}", version));
    }

    let entry_count = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let const_pool_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let code_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;

    let mut offset: usize = 32;

    // Read entry points
    let mut entries = Vec::with_capacity(entry_count);
    let mut name_offs = Vec::new();
    for _ in 0..entry_count {
        if offset + 8 > data.len() {
            return Err("truncated entry points".into());
        }
        let no = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let co = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        name_offs.push(no);
        entries.push(EntryPoint {
            name: String::new(),
            code_offset: co,
        });
        offset += 8;
    }

    // Read entry point names from string data
    for (i, no) in name_offs.iter().enumerate() {
        let mut end = *no;
        while end < data.len() && data[end] != 0 {
            end += 1;
        }
        if end >= data.len() {
            return Err("truncated entry name".into());
        }
        entries[i].name =
            String::from_utf8(data[*no..end].to_vec()).map_err(|e| format!("utf8: {}", e))?;
    }

    // Find end of string data
    let max_name_end = name_offs.iter().map(|&n| n as usize).max().unwrap_or(0);
    let mut string_end = max_name_end;
    while string_end < data.len() && data[string_end] != 0 {
        string_end += 1;
    }
    offset = string_end + 1;

    // Align to 8
    while offset % 8 != 0 {
        offset += 1;
    }
    // Read const pool
    let mut const_pool = Vec::with_capacity(const_pool_count);
    for _ in 0..const_pool_count {
        if offset >= data.len() {
            return Err("truncated const pool".into());
        }
        let tag = data[offset];
        offset += 1;
        match tag {
            0 => {
                if offset + 8 > data.len() {
                    return Err("truncated i64 const".into());
                }
                let v = i64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
                const_pool.push(ConstEntry::I64(v));
                offset += 15; // data(8) + pad(7) [tag already consumed]
            }
            1 => {
                if offset + 8 > data.len() {
                    return Err("truncated f64 const".into());
                }
                let v = f64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
                const_pool.push(ConstEntry::F64(v));
                offset += 15; // data(8) + pad(7) [tag already consumed]
            }
            2 => {
                if offset + 4 > data.len() {
                    return Err("truncated string const len".into());
                }
                let len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
                offset += 4;
                if offset + len > data.len() {
                    return Err("truncated string const data".into());
                }
                let s = String::from_utf8(data[offset..offset + len].to_vec())
                    .map_err(|e| format!("utf8: {}", e))?;
                const_pool.push(ConstEntry::String(s));
                offset += len;
                // align to 8
                while offset % 8 != 0 {
                    offset += 1;
                }
            }
            _ => return Err(format!("bad const pool tag: {}", tag)),
        }
    }

    // Read code section
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

    Ok(QfrProgram {
        entries,
        const_pool,
        code,
        const_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::Opcode;

    #[test]
    fn serialize_deserialize_roundtrip_preserves_entries_consts_and_code() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint {
            name: "on_trade".into(),
            code_offset: 0,
        });
        let s_idx = prog.intern_string("btcusdt");
        let _ = prog.intern_i64(42);
        let _ = prog.intern_f64(3.14);

        prog.code.push(Instruction::rrr(Opcode::Add, 1, 2, 3));
        prog.code.push(Instruction::single(Opcode::Ret));

        let bytes = serialize(&prog);
        assert!(bytes.len() > 32);
        let parsed = deserialize(&bytes).unwrap();
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
        let result = deserialize(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_wrong_magic_bytes_returns_error() {
        let result = deserialize(&[0, 0, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_truncated_header_returns_error() {
        let bytes = b"QFR1";
        let result = deserialize(bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_truncated_code_section_returns_error() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"QFR1");
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        bytes.extend_from_slice(&0u32.to_le_bytes()); // no const pool
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 instruction
        // code section is truncated (8 bytes missing)
        let result = deserialize(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_corrupted_entry_name_returns_error() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"QFR1");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // string data offset
        bytes.extend_from_slice(&20u32.to_le_bytes());
        // entry: name_offset=0, code_offset=0
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // padding
        while bytes.len() < 32 {
            bytes.push(0);
        }
        let result = deserialize(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_deserialize_ten_entries_all_preserved() {
        let mut prog = QfrProgram::new();
        for i in 0..10 {
            prog.entries.push(EntryPoint {
                name: format!("fn{}", i),
                code_offset: i as u32,
            });
        }
        prog.code.push(Instruction::single(Opcode::Halt));
        let bytes = serialize(&prog);
        let parsed = deserialize(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 10);
        assert_eq!(parsed.entries[5].name, "fn5");
    }

    #[test]
    fn serialize_deserialize_hundred_constants_all_preserved() {
        let mut prog = QfrProgram::new();
        for i in 0..100 {
            prog.intern_i64(i);
        }
        prog.code.push(Instruction::single(Opcode::Halt));
        let bytes = serialize(&prog);
        let parsed = deserialize(&bytes).unwrap();
        assert_eq!(parsed.const_pool.len(), 100);
    }

    #[test]
    fn serialize_deserialize_empty_const_pool_succeeds() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code.push(Instruction::single(Opcode::Ret));
        let bytes = serialize(&prog);
        let parsed = deserialize(&bytes).unwrap();
        assert!(parsed.const_pool.is_empty());
        assert_eq!(parsed.code.len(), 1);
    }

    #[test]
    fn deserialize_invalid_const_pool_tag_returns_error() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"QFR1");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 entries
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 const
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 code
        // string data area (empty)
        // const pool: tag = 99 (invalid)
        bytes.push(99);
        let result = deserialize(&bytes);
        assert!(result.is_err());
    }
}
