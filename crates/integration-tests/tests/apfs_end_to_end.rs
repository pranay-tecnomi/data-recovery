//! End-to-end APFS recovery over a synthetic but byte-accurate container.
//!
//! These tests drive the real engine: checkpoint selection, container and
//! volume object maps, B-tree traversal, catalog records, inode and extent
//! reconstruction. Recovered bytes are compared against the known source.

use integration_tests::MemoryDevice;
use integration_tests::apfs_fixture::ApfsImage;
use recovery_core::ByteRange;

fn image() -> (ApfsImage, MemoryDevice) {
    let image = ApfsImage::build();
    let device = MemoryDevice::new(image.bytes.clone());
    (image, device)
}

#[test]
fn discovers_the_container_and_its_volume() {
    let (_image, device) = image();
    let range = device.range();
    let (container, volumes) =
        apfs_recovery::discover_volumes(&device, range).expect("container opens");
    assert_eq!(container.block_size, 4096);
    assert_eq!(volumes.len(), 1, "fixture declares exactly one volume");
}

#[test]
fn resolves_virtual_objects_through_both_object_maps() {
    let (_image, device) = image();
    let range = device.range();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let volume = &volumes[0];
    // The catalog root is a virtual oid; resolving it proves the volume omap
    // B-tree lookup works, which is the core of APFS addressing.
    let physical =
        apfs_recovery::resolve_volume_root(&device, range, &container, &volume.volume, volume.xid)
            .expect("catalog root resolves");
    assert_eq!(physical, integration_tests::apfs_fixture::BLK_CATALOG_ROOT);
}

#[test]
fn indexes_inodes_directory_records_and_extents() {
    let (image, device) = image();
    let range = device.range();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let volume = &volumes[0];
    let index = apfs_recovery::read_volume_filesystem_index(
        &device,
        range,
        &container,
        &volume.volume,
        volume.xid,
    )
    .expect("catalog is readable");

    // Every planned file has an inode; only the linked ones have a name.
    for file in &image.files {
        assert!(
            index.inodes.contains_key(&file.inode_id),
            "inode {} missing from the catalog index",
            file.inode_id
        );
    }
    let linked: Vec<_> = image.files.iter().filter(|f| f.linked).collect();
    assert_eq!(index.directories.len(), linked.len());

    // The fragmented file must have produced more than one extent.
    let report = image.files.iter().find(|f| f.name == "report.txt").unwrap();
    let extents = index
        .extents
        .get(&(report.inode_id + 1000))
        .expect("report.txt has extents");
    assert!(
        extents.len() > 1,
        "report.txt should be fragmented across multiple extents"
    );
}

#[test]
fn recovers_active_file_contents_byte_for_byte() {
    let (image, device) = image();
    let range = device.range();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let volume = &volumes[0];
    let index = apfs_recovery::read_volume_filesystem_index(
        &device,
        range,
        &container,
        &volume.volume,
        volume.xid,
    )
    .unwrap();

    for file in image.files.iter().filter(|f| f.linked) {
        let inode = &index.inodes[&file.inode_id];
        let extents = &index.extents[&inode.private_id];
        let recovered = apfs_recovery::read_file_extents(
            &device,
            range,
            container.block_size,
            extents,
            inode.data_stream_size.unwrap(),
        )
        .expect("extents are readable");
        assert_eq!(
            recovered.len(),
            file.contents.len(),
            "{} recovered to the wrong length",
            file.name
        );
        assert_eq!(
            recovered, file.contents,
            "{} differs from source",
            file.name
        );
    }
}

#[test]
fn recovers_a_deleted_file_from_its_orphaned_inode() {
    let (image, device) = image();
    let range = device.range();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let volume = &volumes[0];
    let index = apfs_recovery::read_volume_filesystem_index(
        &device,
        range,
        &container,
        &volume.volume,
        volume.xid,
    )
    .unwrap();

    let deleted = image.deleted_file();
    // It must be genuinely unreachable from the directory tree...
    assert!(
        !index
            .directories
            .iter()
            .any(|d| d.file_id == deleted.inode_id),
        "the deleted file must have no directory record"
    );
    // ...yet still fully recoverable from metadata, not by carving.
    let inode = &index.inodes[&deleted.inode_id];
    let extents = &index.extents[&inode.private_id];
    let recovered = apfs_recovery::read_file_extents(
        &device,
        range,
        container.block_size,
        extents,
        inode.data_stream_size.unwrap(),
    )
    .expect("deleted file extents are readable");
    assert_eq!(recovered, deleted.contents);
}

#[test]
fn scanning_never_modifies_the_source() {
    let (image, device) = image();
    let range = device.range();
    let before = device.snapshot();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let _ = apfs_recovery::read_volume_filesystem_index(
        &device,
        range,
        &container,
        &volumes[0].volume,
        volumes[0].xid,
    );
    assert_eq!(device.snapshot(), before, "the source image was modified");
    assert_eq!(before, image.bytes);
}

#[test]
fn a_truncated_container_does_not_panic() {
    let image = ApfsImage::build();
    for cut in [512usize, 4096, 8192, 20_000] {
        let device = MemoryDevice::new(image.bytes[..cut].to_vec());
        let Ok(range) = ByteRange::new(0, device.capacity_for_test()) else {
            continue;
        };
        // Any outcome is acceptable except a panic.
        let _ = apfs_recovery::discover_volumes(&device, range);
    }
}

#[test]
fn corrupted_checksums_are_rejected_rather_than_trusted() {
    let mut image = ApfsImage::build();
    // Corrupt the catalog B-tree root's payload without fixing its checksum.
    let at = integration_tests::apfs_fixture::BLK_CATALOG_ROOT as usize * 4096;
    image.bytes[at + 100] ^= 0xFF;
    let device = MemoryDevice::new(image.bytes);
    let range = device.range();
    let (container, volumes) = apfs_recovery::discover_volumes(&device, range).unwrap();
    let result = apfs_recovery::read_volume_filesystem_index(
        &device,
        range,
        &container,
        &volumes[0].volume,
        volumes[0].xid,
    );
    assert!(
        result.is_err(),
        "a block failing its Fletcher-64 checksum must not be trusted"
    );
}
