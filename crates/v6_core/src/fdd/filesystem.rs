use super::image::*;

const MAXCLUST: usize = 390;

/// FDD Filesystem for reading/writing MicroDOS images
pub struct Filesystem {
    pub bytes: Vec<u8>,
}

impl Filesystem {
    /// Create an empty filesystem filled with the empty marker
    pub fn new() -> Self {
        Self {
            bytes: vec![EMPTY_MARKER; FDD_SIZE],
        }
    }

    /// Initialize from existing data
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut bytes = vec![EMPTY_MARKER; FDD_SIZE];
        let len = data.len().min(bytes.len());
        bytes[..len].copy_from_slice(&data[..len]);
        Self { bytes }
    }

    /// Map a CHS sector to its byte offset and return a slice
    pub fn map_sector(&self, track: usize, side: usize, sector: usize) -> &[u8] {
        let sectors = FDD_SECTORS_PER_TRACK * (track * FDD_SIDES + side);
        let sector_adj = if sector > 0 { sector - 1 } else { 0 };
        let offset = (sectors + sector_adj) * FDD_SECTOR_LEN;
        &self.bytes[offset..offset + FDD_SECTOR_LEN]
    }

    fn map_sector_mut(&mut self, track: usize, side: usize, sector: usize) -> &mut [u8] {
        let sectors = FDD_SECTORS_PER_TRACK * (track * FDD_SIDES + side);
        let sector_adj = if sector > 0 { sector - 1 } else { 0 };
        let offset = (sectors + sector_adj) * FDD_SECTOR_LEN;
        &mut self.bytes[offset..offset + FDD_SECTOR_LEN]
    }

    /// Iterate over directory entries
    pub fn read_dir<F>(&self, mut callback: F)
    where
        F: FnMut(&MDHeader) -> bool,
    {
        let mut pos = DIRECTORY_START_OFFSET;
        while pos < DIRECTORY_END_OFFSET {
            let mut header = MDHeader::from_bytes(&self.bytes[pos..pos + ENTRY_SIZE]);
            header.index = (pos - DIRECTORY_START_OFFSET) / ENTRY_SIZE;
            if callback(&header) {
                break;
            }
            pos += ENTRY_SIZE;
        }
    }

    /// Convert cluster number to (track, head, sector)
    pub fn cluster_to_ths(cluster: usize) -> (usize, usize, usize) {
        let mut track = 8 + cluster / 5;
        let head = track % 2;
        track >>= 1;
        let sector = 1 + (cluster % 5);
        (track, head, sector)
    }

    /// Build list of unallocated cluster indices
    pub fn build_available_chain(&self) -> Vec<usize> {
        let mut used = vec![false; MAXCLUST];

        self.read_dir(|header| {
            if header.status <= STATUS_FILE_EXISTS {
                for &ci in &header.fat {
                    if (ci as usize) < used.len() {
                        used[ci as usize] = true;
                    }
                }
            }
            false
        });

        let mut available = Vec::new();
        for i in 2..MAXCLUST {
            if !used[i] {
                available.push(i);
            }
        }
        available
    }

    /// List files in the directory
    pub fn list_files(&self) -> Vec<(String, usize)> {
        let mut files = Vec::new();
        self.read_dir(|header| {
            if header.status <= STATUS_FILE_EXISTS && header.extent == 0 {
                let name = format!("{}.{}", header.filename, header.filetype);
                // Find last extent to compute size
                let size = self.compute_file_size(header);
                files.push((name, size));
            }
            false
        });
        files
    }

    fn compute_file_size(&self, first_header: &MDHeader) -> usize {
        let mut last_extent = first_header.extent;
        let mut last_records = first_header.records;

        self.read_dir(|header| {
            if header.status <= STATUS_FILE_EXISTS
                && header.filename == first_header.filename
                && header.filetype == first_header.filetype
            {
                if header.extent >= last_extent {
                    last_extent = header.extent;
                    last_records = header.records;
                }
            }
            false
        });

        (last_extent as usize) * CLUSTER_LEN * 8 + (last_records as usize) * 128
    }

    /// Save a file to the filesystem.
    /// Returns remaining free space in bytes, or None if disk is full.
    pub fn save_file(&mut self, filename: &str, file_bytes: &[u8]) -> Option<usize> {
        let available = self.build_available_chain();
        let free_space = available.len() * CLUSTER_LEN;

        if free_space < file_bytes.len() {
            return None;
        }

        let header_template = MDHeader::from_name(filename);

        // Track allocation state
        let mut cluster_idx = 0usize;
        let mut extent = 0u8;
        let mut remaining = file_bytes.len();

        // Find free directory slots and allocate
        let mut entries_to_write: Vec<(usize, MDHeader)> = Vec::new();

        let mut pos = DIRECTORY_START_OFFSET;
        while pos < DIRECTORY_END_OFFSET && remaining > 0 {
            let existing = MDHeader::from_bytes(&self.bytes[pos..pos + ENTRY_SIZE]);

            if existing.status >= STATUS_FILE_DOESNT_EXIST {
                let mut h = header_template.clone();
                h.records = ((remaining + 127) / 128).min(RECORD_SIZE as usize) as u8;
                h.extent = extent;
                extent += 1;

                for i in 0..8 {
                    if remaining > 0 && cluster_idx < available.len() {
                        h.fat[i] = available[cluster_idx] as u16;
                        remaining = remaining.saturating_sub(CLUSTER_LEN);
                        cluster_idx += 1;
                    } else {
                        h.fat[i] = 0;
                    }
                }

                entries_to_write.push((pos, h));
            }
            pos += ENTRY_SIZE;
        }

        // Write directory entries
        for (offset, h) in &entries_to_write {
            h.to_bytes(&mut self.bytes[*offset..*offset + ENTRY_SIZE]);
        }

        // Write file data to allocated clusters
        let mut src = 0usize;
        for ci in 0..cluster_idx {
            if src >= file_bytes.len() {
                break;
            }
            let clust = available[ci] << 1;
            for i in 0..2 {
                let (track, head, sector) = Self::cluster_to_ths(clust + i);
                let sect = self.map_sector_mut(track, head, sector);
                for p in 0..FDD_SECTOR_LEN {
                    if src >= file_bytes.len() {
                        break;
                    }
                    sect[p] = file_bytes[src];
                    src += 1;
                }
            }
        }

        // Recalculate free space
        let avail = self.build_available_chain();
        Some(avail.len() * CLUSTER_LEN)
    }
}
