/// Physical FDD geometry constants
pub const FDD_SIDES: usize = 2;
pub const FDD_TRACKS_PER_SIDE: usize = 82;
pub const FDD_SECTORS_PER_TRACK: usize = 5;
pub const FDD_SECTOR_LEN: usize = 1024;
pub const FDD_SIZE: usize = FDD_SIDES * FDD_TRACKS_PER_SIDE * FDD_SECTORS_PER_TRACK * FDD_SECTOR_LEN;

/// File status constants
pub const STATUS_FILE_EXISTS: u8 = 0x0F;
pub const STATUS_FILE_DOESNT_EXIST: u8 = 0x10;
pub const EMPTY_MARKER: u8 = 0xE5;

/// Record size (128 bytes)
pub const RECORD_SIZE: u8 = 0x80;

/// Directory structure constants
pub const DIRECTORY_START_OFFSET: usize = 0xA000;
pub const DIRECTORY_END_OFFSET: usize = 0xB000;
pub const ENTRY_SIZE: usize = 32;
pub const MAX_ENTRIES: usize = (DIRECTORY_END_OFFSET - DIRECTORY_START_OFFSET) / ENTRY_SIZE;

/// Cluster size in bytes
pub const CLUSTER_LEN: usize = 2048;

/// MicroDOS directory header (32 bytes)
#[derive(Debug, Clone)]
pub struct MDHeader {
    pub status: u8,
    pub filename: String,
    pub filetype: String,
    pub extent: u8,
    pub unknown1: u8,
    pub unknown2: u8,
    pub records: u8,
    pub fat: [u16; 8],
    pub index: usize,
}

impl MDHeader {
    pub fn new() -> Self {
        Self {
            status: 0,
            filename: String::new(),
            filetype: String::new(),
            extent: 0,
            unknown1: 0,
            unknown2: 0,
            records: 0,
            fat: [0; 8],
            index: 0,
        }
    }

    /// Parse a 32-byte directory entry
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut h = Self::new();
        h.status = data[0];
        h.filename = String::from_utf8_lossy(&data[1..9]).trim_end().to_string();
        h.filetype = String::from_utf8_lossy(&data[9..12]).trim_end().to_string();
        h.extent = data[12];
        h.unknown1 = data[13];
        h.unknown2 = data[14];
        h.records = data[15];
        for i in 0..8 {
            h.fat[i] = u16::from_le_bytes([data[16 + 2 * i], data[16 + 2 * i + 1]]);
        }
        h
    }

    /// Write header to a 32-byte buffer
    pub fn to_bytes(&self, dest: &mut [u8]) {
        dest[0] = self.status;
        let name = format!("{:<8}", self.filename);
        for (i, b) in name.bytes().take(8).enumerate() {
            dest[1 + i] = b;
        }
        let ext = format!("{:<3}", self.filetype);
        for (i, b) in ext.bytes().take(3).enumerate() {
            dest[9 + i] = b;
        }
        dest[12] = self.extent;
        dest[13] = self.unknown1;
        dest[14] = self.unknown2;
        dest[15] = self.records;
        for i in 0..8 {
            let bytes = self.fat[i].to_le_bytes();
            dest[16 + 2 * i] = bytes[0];
            dest[16 + 2 * i + 1] = bytes[1];
        }
    }

    /// Create header from a filename like "TEST.COM"
    pub fn from_name(filename: &str) -> Self {
        let mut h = Self::new();
        let upper = filename.to_uppercase();
        let parts: Vec<&str> = upper.splitn(2, '.').collect();
        h.filename = parts[0].to_string();
        h.filetype = parts.get(1).unwrap_or(&"").to_string();
        h
    }
}
