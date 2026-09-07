//! Builds a byte-accurate, deterministic APFS container image in memory.
//!
//! The image is assembled from the on-disk structures the recovery engine
//! actually parses - container superblock, checkpoint descriptor, object maps,
//! B-tree nodes and catalog records - with real Fletcher-64 checksums, so a
//! test that recovers a file from it exercises the same code path a real
//! container would. Nothing here is mocked: if the engine's parsing is wrong,
//! these fixtures fail.
//!
//! Layout (block size 4096):
//!   block 0   container superblock (also the checkpoint descriptor base)
//!   block 1   checkpoint container superblock (newest, xid 2)
//!   block 2   container object map
//!   block 3   container object-map B-tree root (fixed kv: volume oid -> block)
//!   block 4   volume superblock
//!   block 5   volume object map
//!   block 6   volume object-map B-tree root (fixed kv: catalog oid -> block)
//!   block 7   catalog B-tree root (variable kv: leaf, holds the records)
//!   block 8+  file data blocks

use apfs_recovery::fletcher64;

pub const BLOCK_SIZE: usize = 4096;
pub const BLOCK_COUNT: u64 = 32;

const NXSB_MAGIC: u32 = 0x4253_584e;
const APSB_MAGIC: u32 = 0x4253_5041;
const OBJ_TYPE_NX_SUPERBLOCK: u32 = 0x0000_0001;
const OBJ_TYPE_BTREE: u32 = 0x0000_0002;
const OBJ_TYPE_SHIFT: u32 = 60;

const APFS_TYPE_INODE: u64 = 3;
const APFS_TYPE_DIR_REC: u64 = 9;
const APFS_TYPE_FILE_EXTENT: u64 = 8;
const INO_EXT_TYPE_DSTREAM: u8 = 8;
const J_INODE_VAL_FIXED_SIZE: usize = 92;

const FLAG_ROOT: u16 = 0x0001;
const FLAG_LEAF: u16 = 0x0002;
const FLAG_FIXED_KV_SIZE: u16 = 0x0004;
const NODE_HEADER_LEN: usize = 56;
const BTREE_FOOTER_LEN: usize = 40;

// Block assignments.
pub const BLK_CHECKPOINT: u64 = 1;
pub const BLK_CONTAINER_OMAP: u64 = 2;
pub const BLK_CONTAINER_OMAP_ROOT: u64 = 3;
pub const BLK_VOLUME_SUPERBLOCK: u64 = 4;
pub const BLK_VOLUME_OMAP: u64 = 5;
pub const BLK_VOLUME_OMAP_ROOT: u64 = 6;
pub const BLK_CATALOG_ROOT: u64 = 7;
pub const BLK_DATA_START: u64 = 8;

/// The virtual object ids used inside the container, resolved via object maps.
pub const VOLUME_OID: u64 = 1024;
pub const CATALOG_OID: u64 = 2048;
pub const XID: u64 = 2;

/// A file placed into the fixture.
pub struct PlannedFile {
    pub name: &'static str,
    pub inode_id: u64,
    pub contents: Vec<u8>,
    /// When false the file has no directory record, which is what deletion
    /// leaves behind: the inode and its extents survive, unreachable.
    pub linked: bool,
}

pub struct ApfsImage {
    pub bytes: Vec<u8>,
    pub files: Vec<PlannedFile>,
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// Writes the object header and stamps a valid Fletcher-64 checksum.
///
/// The checksum must be applied last: it covers every byte after itself.
fn seal(block: &mut [u8], oid: u64, xid: u64, object_type: u32) {
    put_u64(block, 8, oid);
    put_u64(block, 16, xid);
    put_u32(block, 24, object_type);
    put_u32(block, 28, 0);
    let checksum = fletcher64(block).expect("fixture block is checksummable");
    put_u64(block, 0, checksum);
}

/// A jkey: the record type occupies the top 4 bits, the object id the low 60.
fn jkey(record_type: u64, object_id: u64) -> [u8; 8] {
    ((record_type << OBJ_TYPE_SHIFT) | object_id).to_le_bytes()
}

/// Builds a fixed-kv-size B-tree root holding 16-byte omap keys and 16-byte
/// omap values, mapping virtual object ids to physical blocks.
fn omap_btree_root(entries: &[(u64, u64, u64)]) -> Vec<u8> {
    let mut block = vec![0u8; BLOCK_SIZE];
    put_u16(&mut block, 32, FLAG_ROOT | FLAG_LEAF | FLAG_FIXED_KV_SIZE);
    put_u16(&mut block, 34, 0);
    put_u32(&mut block, 36, entries.len() as u32);
    put_u16(&mut block, 40, 0);
    let table_len = (entries.len() * 4).next_multiple_of(8) as u16;
    put_u16(&mut block, 42, table_len);

    let table_start = NODE_HEADER_LEN;
    let kv_start = table_start + table_len as usize;
    // Values are addressed backwards from the end of the value area, which for
    // a root node stops short of the trailing B-tree footer.
    let value_area_end = BLOCK_SIZE - BTREE_FOOTER_LEN;

    for (index, (oid, xid, physical)) in entries.iter().enumerate() {
        let key_offset = index * 16;
        // The reader computes value_start = (area_end - offset) - size, so the
        // offset names the value's END, counted back from the area end.
        let value_offset = index * 16;

        put_u16(&mut block, table_start + index * 4, key_offset as u16);
        put_u16(&mut block, table_start + index * 4 + 2, value_offset as u16);

        let key_at = kv_start + key_offset;
        put_u64(&mut block, key_at, *oid);
        put_u64(&mut block, key_at + 8, *xid);

        let value_start = value_area_end - value_offset - 16;
        put_u32(&mut block, value_start, 0); // flags: not deleted
        put_u32(&mut block, value_start + 4, BLOCK_SIZE as u32);
        put_u64(&mut block, value_start + 8, *physical);
    }
    block
}

/// Builds a variable-kv-size leaf node holding raw catalog records.
fn catalog_leaf(records: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut block = vec![0u8; BLOCK_SIZE];
    put_u16(&mut block, 32, FLAG_ROOT | FLAG_LEAF);
    put_u16(&mut block, 34, 0);
    put_u32(&mut block, 36, records.len() as u32);
    put_u16(&mut block, 40, 0);
    let table_len = (records.len() * 8).next_multiple_of(8) as u16;
    put_u16(&mut block, 42, table_len);

    let table_start = NODE_HEADER_LEN;
    let kv_start = table_start + table_len as usize;
    let value_area_end = BLOCK_SIZE - BTREE_FOOTER_LEN;

    let mut key_cursor = 0usize;
    let mut value_cursor = 0usize;
    for (index, (key, value)) in records.iter().enumerate() {
        let key_offset = key_cursor;
        // The offset names where this value ENDS, counted back from the area
        // end, so it is taken before this value's own length is added.
        let value_offset = value_cursor;

        let entry = table_start + index * 8;
        put_u16(&mut block, entry, key_offset as u16);
        put_u16(&mut block, entry + 2, key.len() as u16);
        put_u16(&mut block, entry + 4, value_offset as u16);
        put_u16(&mut block, entry + 6, value.len() as u16);

        let key_at = kv_start + key_offset;
        block[key_at..key_at + key.len()].copy_from_slice(key);

        let value_start = value_area_end - value_offset - value.len();
        block[value_start..value_start + value.len()].copy_from_slice(value);

        key_cursor += key.len();
        value_cursor += value.len();
    }
    block
}

/// An inode value carrying a DSTREAM xfield, which is where the real file
/// size lives for a regular file.
fn inode_value(parent_id: u64, private_id: u64, mode: u16, size: u64) -> Vec<u8> {
    let mut value = vec![0u8; J_INODE_VAL_FIXED_SIZE];
    put_u64(&mut value, 0, parent_id);
    put_u64(&mut value, 8, private_id);
    put_u32(&mut value, 56, 1); // nlink
    put_u16(&mut value, 80, mode);
    put_u64(&mut value, 84, size); // uncompressed size

    // xfield blob: one DSTREAM descriptor followed by its 40-byte value.
    let dstream_len = 40usize;
    let mut x = Vec::new();
    x.extend_from_slice(&1u16.to_le_bytes()); // xfield count
    x.extend_from_slice(&(dstream_len as u16).to_le_bytes()); // used data
    x.push(INO_EXT_TYPE_DSTREAM);
    x.push(0);
    x.extend_from_slice(&(dstream_len as u16).to_le_bytes());
    let mut dstream = vec![0u8; dstream_len];
    put_u64(&mut dstream, 0, size); // j_dstream.size
    x.extend_from_slice(&dstream);
    value.extend_from_slice(&x);
    value
}

impl ApfsImage {
    /// Builds a container holding one volume with three files: two linked into
    /// the directory tree (one of them fragmented across non-adjacent blocks)
    /// and one orphaned inode standing in for a deleted file.
    pub fn build() -> Self {
        let files = vec![
            PlannedFile {
                name: "report.txt",
                inode_id: 20,
                contents: {
                    // Spans two blocks so extent reassembly is exercised.
                    let mut v = Vec::new();
                    for i in 0..(BLOCK_SIZE + 1000) {
                        v.push((i % 251) as u8);
                    }
                    v
                },
                linked: true,
            },
            PlannedFile {
                name: "notes.txt",
                inode_id: 21,
                contents: b"the quick brown fox jumps over the lazy dog".to_vec(),
                linked: true,
            },
            PlannedFile {
                name: "deleted.bin",
                inode_id: 22,
                contents: {
                    let mut v = Vec::new();
                    for i in 0..3000 {
                        v.push((i % 97) as u8);
                    }
                    v
                },
                linked: false,
            },
        ];

        let mut bytes = vec![0u8; BLOCK_SIZE * BLOCK_COUNT as usize];

        // --- file data, and the extent records describing it ---
        let mut records: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut next_block = BLK_DATA_START;

        // Records must be emitted in ascending key order for a realistic tree.
        let mut extent_records: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut inode_records: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut drec_records: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

        for file in &files {
            let dstream_id = file.inode_id + 1000;
            let size = file.contents.len() as u64;

            // Lay the file out one block at a time, leaving a gap between the
            // blocks of the first file so its extents are genuinely fragmented.
            let mut logical = 0u64;
            let mut written = 0usize;
            while written < file.contents.len() {
                let chunk = (file.contents.len() - written).min(BLOCK_SIZE);
                let physical = next_block;
                next_block += 1;
                if file.name == "report.txt" {
                    // Skip a block to force non-contiguous placement.
                    next_block += 1;
                }
                let at = physical as usize * BLOCK_SIZE;
                bytes[at..at + chunk].copy_from_slice(&file.contents[written..written + chunk]);

                // A FILE_EXTENT record: key is jkey(dstream_id) + logical addr.
                let mut key = Vec::new();
                key.extend_from_slice(&jkey(APFS_TYPE_FILE_EXTENT, dstream_id));
                key.extend_from_slice(&logical.to_le_bytes());
                let mut value = vec![0u8; 24];
                // Extent lengths are block-aligned on disk; the inode size is
                // what truncates the tail.
                put_u64(&mut value, 0, BLOCK_SIZE as u64);
                put_u64(&mut value, 8, physical);
                extent_records.push((key, value));

                logical += BLOCK_SIZE as u64;
                written += chunk;
            }

            // The inode record.
            let mut key = Vec::new();
            key.extend_from_slice(&jkey(APFS_TYPE_INODE, file.inode_id));
            inode_records.push((key, inode_value(2, dstream_id, 0o100644, size)));

            // The directory record that names it, unless it is "deleted".
            if file.linked {
                let name_bytes = {
                    let mut n = file.name.as_bytes().to_vec();
                    n.push(0);
                    n
                };
                let mut key = Vec::new();
                key.extend_from_slice(&jkey(APFS_TYPE_DIR_REC, 2)); // parent = root
                key.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                key.extend_from_slice(&name_bytes);
                let mut value = vec![0u8; 18];
                put_u64(&mut value, 0, file.inode_id);
                drec_records.push((key, value));
            }
        }

        records.extend(inode_records);
        records.extend(extent_records);
        records.extend(drec_records);

        // --- catalog B-tree ---
        let catalog = catalog_leaf(&records);
        let at = BLK_CATALOG_ROOT as usize * BLOCK_SIZE;
        bytes[at..at + BLOCK_SIZE].copy_from_slice(&catalog);
        seal(
            &mut bytes[at..at + BLOCK_SIZE],
            CATALOG_OID,
            XID,
            OBJ_TYPE_BTREE,
        );

        // --- volume object map: catalog virtual oid -> physical block ---
        let vroot = omap_btree_root(&[(CATALOG_OID, XID, BLK_CATALOG_ROOT)]);
        let at = BLK_VOLUME_OMAP_ROOT as usize * BLOCK_SIZE;
        bytes[at..at + BLOCK_SIZE].copy_from_slice(&vroot);
        seal(&mut bytes[at..at + BLOCK_SIZE], 0, XID, OBJ_TYPE_BTREE);

        let at = BLK_VOLUME_OMAP as usize * BLOCK_SIZE;
        {
            let b = &mut bytes[at..at + BLOCK_SIZE];
            put_u32(b, 40, OBJ_TYPE_BTREE); // tree type
            put_u32(b, 44, OBJ_TYPE_BTREE); // snapshot tree type
            put_u64(b, 48, BLK_VOLUME_OMAP_ROOT);
        }
        seal(&mut bytes[at..at + BLOCK_SIZE], 0, XID, 0);

        // --- volume superblock ---
        let at = BLK_VOLUME_SUPERBLOCK as usize * BLOCK_SIZE;
        {
            let b = &mut bytes[at..at + BLOCK_SIZE];
            put_u32(b, 32, APSB_MAGIC);
            put_u64(b, 128, BLK_VOLUME_OMAP); // omap is a physical oid
            put_u64(b, 136, CATALOG_OID); // root tree is virtual
        }
        seal(&mut bytes[at..at + BLOCK_SIZE], VOLUME_OID, XID, 0);

        // --- container object map: volume virtual oid -> physical block ---
        let croot = omap_btree_root(&[(VOLUME_OID, XID, BLK_VOLUME_SUPERBLOCK)]);
        let at = BLK_CONTAINER_OMAP_ROOT as usize * BLOCK_SIZE;
        bytes[at..at + BLOCK_SIZE].copy_from_slice(&croot);
        seal(&mut bytes[at..at + BLOCK_SIZE], 0, XID, OBJ_TYPE_BTREE);

        let at = BLK_CONTAINER_OMAP as usize * BLOCK_SIZE;
        {
            let b = &mut bytes[at..at + BLOCK_SIZE];
            put_u32(b, 40, OBJ_TYPE_BTREE);
            put_u32(b, 44, OBJ_TYPE_BTREE);
            put_u64(b, 48, BLK_CONTAINER_OMAP_ROOT);
        }
        seal(&mut bytes[at..at + BLOCK_SIZE], 0, XID, 0);

        // --- container superblocks (block 0, and the newer checkpoint) ---
        for (block, xid) in [(0u64, 1u64), (BLK_CHECKPOINT, XID)] {
            let at = block as usize * BLOCK_SIZE;
            let b = &mut bytes[at..at + BLOCK_SIZE];
            put_u32(b, 32, NXSB_MAGIC);
            put_u32(b, 36, BLOCK_SIZE as u32);
            put_u64(b, 40, BLOCK_COUNT);
            put_u64(b, 176, BLK_CONTAINER_OMAP);
            // Checkpoint descriptor ring: 2 blocks starting at block 0.
            put_u32(b, 0x68, 2); // xp_desc_blocks
            put_u64(b, 0x70, 0); // xp_desc_base
            put_u32(b, 0x88, block as u32); // xp_desc_index
            put_u32(b, 0x8c, 1); // xp_desc_len
            // Volume oid table.
            put_u32(b, 0xC4, 1); // max file systems
            put_u64(b, 0xC8, VOLUME_OID);
            seal(b, block, xid, OBJ_TYPE_NX_SUPERBLOCK);
        }

        Self { bytes, files }
    }

    /// The file the fixture treats as deleted (an inode with no directory record).
    pub fn deleted_file(&self) -> &PlannedFile {
        self.files
            .iter()
            .find(|f| !f.linked)
            .expect("fixture has a deleted file")
    }
}
